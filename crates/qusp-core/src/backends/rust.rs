//! Rust backend — native installer.
//!
//! Toolchains come from `static.rust-lang.org`, the same CDN that
//! powers `rustup`. For a pin like `rust = "1.78.0"`, qusp downloads
//! `rust-1.78.0-<triple>.tar.gz` (the unified installer bundle) and
//! verifies it against the matching `.sha256` sidecar.
//!
//! The unified tarball is structured as one directory per component
//! (`rustc/`, `cargo/`, `rust-std-<triple>/`, …), each laying out its
//! own `bin/`, `lib/`, `share/` subtree. Upstream's `install.sh` would
//! merge those into a flat layout. We re-implement that merge in Rust
//! (skipping `manifest.in` and `components` markers) so the install is
//! purely Rust + filesystem — **no subprocess freeloading** on
//! `install.sh` or `rustup`.
//!
//! Tools are intentionally empty for v0.7.0. Cargo's own
//! `cargo install` (and `cargo binstall` if pinned manually under
//! `[rust.tools]`) handles the Rust ecosystem more honestly than a
//! curated registry would.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct RustBackend;

const DIST_BASE: &str = "https://static.rust-lang.org/dist";

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn rust_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("rust").join(version)
}

#[async_trait]
impl Backend for RustBackend {
    fn id(&self) -> &'static str {
        "rust"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["rust-toolchain.toml", "rust-toolchain", "Cargo.toml"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            // Plain text `rust-toolchain` first (asdf-compatible).
            let plain = d.join("rust-toolchain");
            if plain.is_file() {
                let raw = std::fs::read_to_string(&plain).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() && !v.contains('=') {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: "rust-toolchain".into(),
                        origin: plain,
                    }));
                }
            }
            // TOML form: [toolchain] channel = "1.78.0"
            let toml_form = d.join("rust-toolchain.toml");
            if toml_form.is_file() {
                let raw = std::fs::read_to_string(&toml_form).unwrap_or_default();
                if let Some(channel) = parse_toolchain_channel(&raw) {
                    return Ok(Some(DetectedVersion {
                        version: channel,
                        source: "rust-toolchain.toml".into(),
                        origin: toml_form,
                    }));
                }
            }
            dir = d.parent();
        }
        Ok(None)
    }

    async fn install(
        &self,
        _: &AnyvPaths,
        version: &str,
        _opts: &InstallOpts,
        http: &dyn crate::effects::HttpFetcher,
        _progress: &dyn crate::effects::ProgressReporter,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = rust_root(&paths, version);
        if install_dir.join("bin").join("rustc").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard = crate::effects::StoreLock::acquire(
            &crate::effects::lock_path_for(&install_dir),
        )?;
        let triple = target_triple()
            .ok_or_else(|| anyhow!("static.rust-lang.org has no asset for this platform"))?;
        // Resolve channel names (stable/beta/nightly) to a concrete version
        // by reading the channel manifest.
        let resolved_version = if matches!(version, "stable" | "beta" | "nightly") {
            resolve_channel(http, version).await?
        } else {
            version.to_string()
        };
        let asset = format!("rust-{resolved_version}-{triple}.tar.gz");
        let asset_url = format!("{DIST_BASE}/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = sha_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty sha256 sidecar for {asset}"))?
            .to_string();

        let bytes = http
            .get_bytes(&asset_url)
            .await
            .with_context(|| format!("download {asset_url}"))?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if expected != actual {
            bail!("sha256 mismatch for {asset}: expected {expected}, got {actual}");
        }

        let cache_path = paths.cache.join(&asset);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        // Find the unified top-level dir, then merge each component into a
        // single install prefix. This is what install.sh would have done.
        let top = find_unified_top(&store_dir)?;
        let merged_root = store_dir.join("merged");
        anyv_core::paths::ensure_dir(&merged_root)?;
        merge_components(&top, &merged_root)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&merged_root, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), merged_root.display())
            })?;

        Ok(InstallReport {
            version: resolved_version,
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = rust_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("rust {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("rust");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            // Skip the install lock files written by `StoreLock::acquire`.
            if name.ends_with(".qusp-lock") {
                continue;
            }
            out.push(name);
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        // Resolved stable first so consumers (e.g. `qusp outdated`) treat
        // the concrete version as "newest". Channel pointers follow for
        // human reference.
        let stable = resolve_channel(http, "stable").await.unwrap_or_default();
        let mut out = Vec::new();
        if !stable.is_empty() {
            out.push(stable);
        }
        out.extend(["stable".into(), "beta".into(), "nightly".into()]);
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        _http: &dyn crate::effects::HttpFetcher,
        name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "Rust ecosystem tools are managed by `cargo install` / `cargo binstall`. \
             qusp doesn't curate a Rust tool registry — pin under [rust.tools] manually \
             with explicit `package = \"…\"` if you need it tracked. '{name}' has no \
             qusp-managed install path."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = rust_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("RUSTUP_TOOLCHAIN".into(), version.to_string());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }
}

/// Resolve `stable`/`beta`/`nightly` to a concrete version string by
/// fetching the channel manifest. The manifest is laid out as:
///   `[pkg.cargo]` section, then `[pkg.rust]` section, then `[pkg.rustc]`
///   section, each with its own `version = "X.Y.Z (commit-sha date)"`.
/// We want `[pkg.rust]`'s version, which mirrors what `rustup install
/// stable` resolves to.
async fn resolve_channel(http: &dyn crate::effects::HttpFetcher, channel: &str) -> Result<String> {
    let url = format!("{DIST_BASE}/channel-rust-{channel}.toml");
    let body = http
        .get_text(&url)
        .await
        .with_context(|| format!("fetch {url}"))?;
    parse_channel_rust_version(&body, channel).ok_or_else(|| {
        anyhow!("could not parse [pkg.rust] version from channel-rust-{channel}.toml")
    })
}

/// Pure: scan a `channel-rust-<channel>.toml` body for `[pkg.rust]` and
/// return the bare version (e.g. `1.95.0`).
pub(crate) fn parse_channel_rust_version(body: &str, _channel: &str) -> Option<String> {
    let mut in_rust_section = false;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_rust_section = line == "[pkg.rust]";
            continue;
        }
        if !in_rust_section {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version = \"") {
            if let Some(end) = rest.find('"') {
                let raw = &rest[..end];
                // raw looks like `1.85.0 (commit-sha YYYY-MM-DD)`.
                if let Some(v) = raw.split_whitespace().next() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// The unified Rust tarball expands to one top-level directory like
/// `rust-1.78.0-<triple>/`. Find it.
fn find_unified_top(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let p = e.path();
            // The unified top-level dir contains a `components` text file.
            if p.join("components").is_file() {
                return Ok(p);
            }
        }
    }
    bail!(
        "no unified Rust top-level dir (with `components` marker) inside {}",
        store_dir.display()
    )
}

/// Merge each component subtree (`rustc/`, `cargo/`, `rust-std-…/`, …)
/// into a single install prefix. This is what `install.sh` does in
/// shell; we do it in Rust.
fn merge_components(top: &Path, dest: &Path) -> Result<()> {
    let components = std::fs::read_to_string(top.join("components"))
        .with_context(|| format!("read {}", top.join("components").display()))?;
    for comp in components.lines() {
        let comp = comp.trim();
        if comp.is_empty() {
            continue;
        }
        let comp_dir = top.join(comp);
        if !comp_dir.is_dir() {
            // Not all listed components ship in every channel/host; skip.
            continue;
        }
        copy_tree(&comp_dir, dest)?;
    }
    Ok(())
}

/// Recursively copy `src/**` into `dest/**`, skipping the per-component
/// `manifest.in` markers.
fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let from = e.path();
        let name = e.file_name();
        // Skip per-component metadata.
        if name == "manifest.in" {
            continue;
        }
        let to = dest.join(&name);
        let ft = e.file_type()?;
        if ft.is_dir() {
            anyv_core::paths::ensure_dir(&to)?;
            copy_tree(&from, &to)?;
        } else if ft.is_symlink() {
            let target = std::fs::read_link(&from)?;
            // Idempotent: if a symlink with the same target already exists, skip.
            if to.exists() || to.is_symlink() {
                let _ = std::fs::remove_file(&to);
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &to)?;
            #[cfg(windows)]
            {
                let _ = target;
                std::fs::copy(&from, &to)?;
            }
        } else {
            // First component to deposit a file wins; subsequent components
            // skip (matches install.sh semantics where `rustc` ships the
            // base files and overlays add new ones).
            if !to.exists() {
                std::fs::copy(&from, &to)?;
                #[cfg(unix)]
                {
                    let perms = std::fs::metadata(&from)?.permissions();
                    std::fs::set_permissions(&to, perms)?;
                }
            }
        }
    }
    Ok(())
}

/// Pull the `channel = "..."` value out of a rust-toolchain.toml.
fn parse_toolchain_channel(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("channel") {
            let rest = rest.trim_start();
            let rest = rest.strip_prefix('=')?.trim_start();
            let rest = rest.strip_prefix('"')?;
            let end = rest.find('"')?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.split_whitespace().next().unwrap_or(s);
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_manifest_parser_skips_pkg_cargo() {
        let body = r#"manifest-version = "2"
date = "2026-04-16"

[pkg.cargo]
version = "0.96.0 (f2d3ce0bd 2026-03-21)"

[pkg.rust]
version = "1.95.0 (abc 2026-04-16)"
"#;
        assert_eq!(
            parse_channel_rust_version(body, "stable").as_deref(),
            Some("1.95.0")
        );
    }

    #[test]
    fn channel_manifest_returns_none_when_section_missing() {
        let body = r#"manifest-version = "2"

[pkg.cargo]
version = "0.96.0"
"#;
        assert!(parse_channel_rust_version(body, "stable").is_none());
    }

    #[test]
    fn rust_toolchain_toml_channel_extracted() {
        let raw = "[toolchain]\nchannel = \"1.78.0\"\n";
        assert_eq!(parse_toolchain_channel(raw).as_deref(), Some("1.78.0"));
    }
}
