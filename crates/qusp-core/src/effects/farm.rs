//! Symlink farm — `~/.local/bin/python3.13` style global PATH entries.
//!
//! Phase 1 dogfood / "PC-wide" use case: when the user installs a
//! toolchain via `qusp install`, qusp materialises symlinks in a
//! global `bin/` dir so the binaries are reachable as **bare commands**
//! without any project context, manifest, or shell hook activation.
//!
//! This is the same model uv uses for `~/.local/bin/python3.13`:
//! a thin symlink farm that sits next to the content-addressed store
//! and gives users the "system-installed" feel for qusp-managed
//! toolchains. No shim, no overhead — just `readlink` resolution.
//!
//! ## Two flavours of binary
//!
//! - **Versioned** (`python3.13`, `ruby3.4`): always created on install.
//!   Multiple versions co-exist with no conflict.
//! - **Unversioned** (`python`, `cargo`, `scala`): created **only when
//!   the user has explicitly set a global pin** for that language.
//!   Without a pin, qusp doesn't claim the bare command name —
//!   leaves it to the user (system / brew / mise / etc.).
//!
//! Each backend declares its own `Vec<FarmBinary>` via
//! [`Backend::farm_binaries`].
//!
//! ## Conflict policy
//!
//! When a target symlink already exists and points outside the qusp
//! store (e.g. uv has placed `python3.13` already), qusp's default
//! is **non-destructive**: log a warning, skip the link. The user
//! sees the conflict in `qusp doctor` and can resolve it with
//! `qusp pin --global` + a manual `rm` + reinstall, or by toggling
//! `--override-foreign-links` (future flag).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One bin a backend wants exposed in the global symlink farm.
///
/// `name` is the literal symlink filename inside `farm_dir/`. For
/// versioned binaries this includes the version suffix
/// (`python3.13`); for unversioned ones it's the bare command name
/// (`python`, `cargo`).
#[derive(Debug, Clone)]
pub struct FarmBinary {
    /// Path inside the install_dir's `bin/` (or wherever the binary
    /// lives). Usually a bare filename (`python3.13`).
    pub source: String,
    /// Target name in the farm dir. Often the same as `source`, but
    /// can differ (e.g. for renamed exposure).
    pub link_name: String,
    /// Whether this is a version-suffixed name (always exposed) or a
    /// bare name (exposed only when global-pinned).
    pub kind: FarmKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarmKind {
    /// `python3.13`, `ruby3.4` — exposed unconditionally on install.
    Versioned,
    /// `python`, `cargo`, `scala` — exposed only when the global pin
    /// for this lang says so.
    Unversioned,
}

impl FarmBinary {
    /// Versioned binary living under `bin/` (e.g. `bin/python3.13`).
    pub fn versioned(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            source: format!("bin/{name}"),
            link_name: name,
            kind: FarmKind::Versioned,
        }
    }
    /// Unversioned binary living under `bin/` (e.g. `bin/python`).
    pub fn unversioned(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            source: format!("bin/{name}"),
            link_name: name,
            kind: FarmKind::Unversioned,
        }
    }
    /// Versioned binary at install root (flat layout, e.g. `zig`).
    pub fn versioned_flat(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            source: name.clone(),
            link_name: name,
            kind: FarmKind::Versioned,
        }
    }
    /// Unversioned binary at install root (flat layout, e.g. `zig`).
    pub fn unversioned_flat(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            source: name.clone(),
            link_name: name,
            kind: FarmKind::Unversioned,
        }
    }
}

/// Manages the global symlink farm at `~/.local/bin/` (or override).
pub struct FarmManager {
    pub farm_dir: PathBuf,
}

impl FarmManager {
    /// Default farm dir: `$HOME/.local/bin/` — matches XDG and uv
    /// convention. If `HOME` is unset, falls back to `/tmp/qusp-bin`.
    pub fn default() -> Self {
        let dir = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".local").join("bin"))
            .unwrap_or_else(|| PathBuf::from("/tmp/qusp-bin"));
        Self { farm_dir: dir }
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { farm_dir: dir }
    }

    /// Materialise versioned + (optionally) unversioned symlinks for
    /// a backend's binaries. The unversioned set is filtered by
    /// `expose_unversioned` (true when the global pin for this lang
    /// matches the version being installed).
    ///
    /// Existing links pointing at the qusp store are silently
    /// replaced (atomic via `atomic_symlink_swap`). Links pointing
    /// outside the qusp store (e.g. uv's symlinks) are **left alone**
    /// and a warning is logged — qusp won't clobber another tool's
    /// state.
    pub fn install_links(
        &self,
        install_dir: &Path,
        bins: &[FarmBinary],
        expose_unversioned: bool,
        store_root: &Path,
    ) -> Result<FarmReport> {
        anyv_core::paths::ensure_dir(&self.farm_dir)?;
        let mut report = FarmReport::default();
        for bin in bins {
            if bin.kind == FarmKind::Unversioned && !expose_unversioned {
                continue;
            }
            let source = install_dir.join(&bin.source);
            if !source.is_file() && !source.is_symlink() {
                report.skipped_missing.push(bin.link_name.clone());
                continue;
            }
            let link = self.farm_dir.join(&bin.link_name);
            if let Some(existing) = read_link_target(&link) {
                if !existing.starts_with(store_root) {
                    // Foreign link (uv, brew, etc.) — preserve.
                    report.skipped_foreign.push((
                        bin.link_name.clone(),
                        existing.to_string_lossy().into_owned(),
                    ));
                    tracing::warn!(
                        "farm: leaving foreign symlink at {} → {} alone",
                        link.display(),
                        existing.display()
                    );
                    continue;
                }
            } else if link.exists() {
                // Regular file or directory at the link path — also foreign.
                report.skipped_foreign.push((
                    bin.link_name.clone(),
                    "non-symlink (regular file or dir)".into(),
                ));
                continue;
            }
            super::install_lock::atomic_symlink_swap(&source, &link)
                .with_context(|| format!("farm: link {} → {}", link.display(), source.display()))?;
            report.linked.push(bin.link_name.clone());
        }
        Ok(report)
    }

    /// Remove farm symlinks that point into a specific install dir.
    /// Used by `qusp uninstall` to clean up after a toolchain removal.
    /// Foreign symlinks are left alone (same policy as install_links).
    pub fn remove_links_to(&self, install_dir: &Path) -> Result<usize> {
        if !self.farm_dir.is_dir() {
            return Ok(0);
        }
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.farm_dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(target) = read_link_target(&path) {
                if target.starts_with(install_dir) {
                    if std::fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }

    /// Inspect: list every symlink in the farm dir whose target lives
    /// inside `store_root`. Used by `qusp doctor`.
    pub fn list_qusp_links(&self, store_root: &Path) -> Vec<FarmEntry> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.farm_dir) else {
            return out;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Some(target) = read_link_target(&path) else {
                continue;
            };
            let target_in_store = target.starts_with(store_root);
            let target_exists = target.exists();
            out.push(FarmEntry {
                link: path,
                target,
                qusp_owned: target_in_store,
                target_alive: target_exists,
            });
        }
        out
    }
}

#[derive(Debug, Default)]
pub struct FarmReport {
    /// Successfully created or refreshed.
    pub linked: Vec<String>,
    /// Skipped because source bin doesn't exist in install_dir.
    pub skipped_missing: Vec<String>,
    /// Skipped because target path is held by a foreign tool
    /// (link not pointing into qusp store, or non-symlink file).
    pub skipped_foreign: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct FarmEntry {
    pub link: PathBuf,
    pub target: PathBuf,
    /// True iff target is inside qusp's store_root.
    pub qusp_owned: bool,
    /// True iff target file/dir still exists.
    pub target_alive: bool,
}

/// Global pin: which lang+version owns the unversioned bare command
/// in the farm dir. Stored at `paths.config/global.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalPins {
    /// backend id → pinned version (the version whose unversioned
    /// binaries get exposed in the farm dir).
    #[serde(default, flatten)]
    pub pins: std::collections::BTreeMap<String, GlobalPin>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalPin {
    pub version: String,
    /// Optional vendor selector (Java distribution).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub distribution: String,
}

impl GlobalPins {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let p = config_dir.join("global.toml");
        if !p.is_file() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", p.display()))
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        anyv_core::paths::ensure_dir(config_dir)?;
        let p = config_dir.join("global.toml");
        let text =
            toml::to_string_pretty(self).with_context(|| format!("serialize {}", p.display()))?;
        std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))?;
        Ok(())
    }

    pub fn get(&self, lang: &str) -> Option<&GlobalPin> {
        self.pins.get(lang)
    }

    pub fn set(&mut self, lang: &str, version: &str, distribution: Option<&str>) {
        self.pins.insert(
            lang.to_string(),
            GlobalPin {
                version: version.to_string(),
                distribution: distribution.unwrap_or("").to_string(),
            },
        );
    }

    pub fn remove(&mut self, lang: &str) -> Option<GlobalPin> {
        self.pins.remove(lang)
    }
}

fn read_link_target(p: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(p).ok()?;
    if !meta.file_type().is_symlink() {
        return None;
    }
    std::fs::read_link(p).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "qusp-farm-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        anyv_core::paths::ensure_dir(&p).unwrap();
        p
    }

    #[test]
    fn versioned_link_created_on_install() {
        let store = tmp_dir("store");
        let install = store.join("python-3.13");
        anyv_core::paths::ensure_dir(&install.join("bin")).unwrap();
        std::fs::write(install.join("bin/python3.13"), "#!/bin/sh\n").unwrap();

        let farm_dir = tmp_dir("farm");
        let manager = FarmManager::with_dir(farm_dir.clone());
        let report = manager
            .install_links(
                &install,
                &[FarmBinary::versioned("python3.13")],
                false, // expose_unversioned: false (no global pin)
                &store,
            )
            .unwrap();
        assert_eq!(report.linked, vec!["python3.13".to_string()]);
        let link = farm_dir.join("python3.13");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            install.join("bin/python3.13")
        );

        std::fs::remove_dir_all(&store).ok();
        std::fs::remove_dir_all(&farm_dir).ok();
    }

    #[test]
    fn unversioned_link_skipped_without_pin_exposed() {
        let store = tmp_dir("store");
        let install = store.join("python-3.13");
        anyv_core::paths::ensure_dir(&install.join("bin")).unwrap();
        std::fs::write(install.join("bin/python"), "#!/bin/sh\n").unwrap();

        let farm_dir = tmp_dir("farm");
        let manager = FarmManager::with_dir(farm_dir.clone());
        let report = manager
            .install_links(
                &install,
                &[FarmBinary::unversioned("python")],
                false, // no pin → skip
                &store,
            )
            .unwrap();
        assert!(report.linked.is_empty());
        assert!(!farm_dir.join("python").exists());

        std::fs::remove_dir_all(&store).ok();
        std::fs::remove_dir_all(&farm_dir).ok();
    }

    #[test]
    fn unversioned_link_created_when_pin_exposed() {
        let store = tmp_dir("store");
        let install = store.join("python-3.13");
        anyv_core::paths::ensure_dir(&install.join("bin")).unwrap();
        std::fs::write(install.join("bin/python"), "#!/bin/sh\n").unwrap();

        let farm_dir = tmp_dir("farm");
        let manager = FarmManager::with_dir(farm_dir.clone());
        let report = manager
            .install_links(
                &install,
                &[FarmBinary::unversioned("python")],
                true, // global pin says python = 3.13
                &store,
            )
            .unwrap();
        assert_eq!(report.linked, vec!["python".to_string()]);
        assert_eq!(
            std::fs::read_link(farm_dir.join("python")).unwrap(),
            install.join("bin/python")
        );

        std::fs::remove_dir_all(&store).ok();
        std::fs::remove_dir_all(&farm_dir).ok();
    }

    #[test]
    fn foreign_symlink_preserved() {
        // Simulate uv having claimed `python3.13` already.
        let store = tmp_dir("store");
        let install = store.join("python-3.13");
        anyv_core::paths::ensure_dir(&install.join("bin")).unwrap();
        std::fs::write(install.join("bin/python3.13"), "#!/bin/sh\n").unwrap();

        let foreign_root = tmp_dir("foreign");
        let foreign_target = foreign_root.join("uv-managed-python3.13");
        std::fs::write(&foreign_target, "uv\n").unwrap();

        let farm_dir = tmp_dir("farm");
        // Pre-place a foreign symlink that doesn't point into qusp store.
        std::os::unix::fs::symlink(&foreign_target, farm_dir.join("python3.13")).unwrap();

        let manager = FarmManager::with_dir(farm_dir.clone());
        let report = manager
            .install_links(
                &install,
                &[FarmBinary::versioned("python3.13")],
                false,
                &store,
            )
            .unwrap();
        assert!(report.linked.is_empty(), "should not have linked");
        assert_eq!(report.skipped_foreign.len(), 1);
        // The foreign symlink survives untouched.
        assert_eq!(
            std::fs::read_link(farm_dir.join("python3.13")).unwrap(),
            foreign_target
        );

        std::fs::remove_dir_all(&store).ok();
        std::fs::remove_dir_all(&foreign_root).ok();
        std::fs::remove_dir_all(&farm_dir).ok();
    }

    #[test]
    fn remove_links_to_cleans_only_qusp_owned() {
        let store = tmp_dir("store");
        let install = store.join("python-3.13");
        anyv_core::paths::ensure_dir(&install.join("bin")).unwrap();
        std::fs::write(install.join("bin/python3.13"), "#!/bin/sh\n").unwrap();

        let foreign_root = tmp_dir("foreign");
        let foreign_target = foreign_root.join("foreign-bin");
        std::fs::write(&foreign_target, "x\n").unwrap();

        let farm_dir = tmp_dir("farm");
        anyv_core::paths::ensure_dir(&farm_dir).unwrap();
        std::os::unix::fs::symlink(install.join("bin/python3.13"), farm_dir.join("python3.13"))
            .unwrap();
        std::os::unix::fs::symlink(&foreign_target, farm_dir.join("foreign-tool")).unwrap();

        let manager = FarmManager::with_dir(farm_dir.clone());
        let removed = manager.remove_links_to(&install).unwrap();
        assert_eq!(removed, 1);
        assert!(!farm_dir.join("python3.13").exists());
        assert!(farm_dir.join("foreign-tool").exists()); // preserved

        std::fs::remove_dir_all(&store).ok();
        std::fs::remove_dir_all(&foreign_root).ok();
        std::fs::remove_dir_all(&farm_dir).ok();
    }

    #[test]
    fn global_pins_round_trip_via_toml() {
        let dir = tmp_dir("config");
        let mut pins = GlobalPins::default();
        pins.set("python", "3.13.0", None);
        pins.set("java", "21", Some("temurin"));
        pins.save(&dir).unwrap();

        let loaded = GlobalPins::load(&dir).unwrap();
        assert_eq!(loaded.get("python").unwrap().version, "3.13.0");
        assert_eq!(loaded.get("java").unwrap().version, "21");
        assert_eq!(loaded.get("java").unwrap().distribution, "temurin");
        assert!(loaded.get("ruby").is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn global_pin_remove_drops_entry() {
        let mut pins = GlobalPins::default();
        pins.set("python", "3.13.0", None);
        let removed = pins.remove("python");
        assert!(removed.is_some());
        assert!(pins.get("python").is_none());
    }
}
