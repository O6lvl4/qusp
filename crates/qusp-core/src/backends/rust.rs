//! Rust backend — native installer.
//!
//! Toolchains come from `static.rust-lang.org`, the same CDN that
//! powers `rustup`. qusp downloads the unified tarball, verifies
//! against the `.sha256` sidecar, and merges components (rustc,
//! cargo, rust-std, …) into a single install prefix — no subprocess
//! freeloading on `install.sh` or `rustup`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;

use super::common;
use crate::backend::*;

pub struct RustBackend;

const DIST_BASE: &str = "https://static.rust-lang.org/dist";

fn target_triple() -> Option<&'static str> {
    Some(match common::os_arch() {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
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
        ctx: &InstallCtx<'_>,
    ) -> Result<InstallReport> {
        let paths = common::qusp_paths()?;
        paths.ensure_dirs()?;
        let install_dir = common::lang_root(&paths, "rust", version);

        if let Some(report) = common::check_already_installed(&install_dir, "bin/rustc", version) {
            return Ok(report);
        }
        let _guard = common::acquire_install_lock(&install_dir)?;

        let triple = target_triple()
            .ok_or_else(|| anyhow!("static.rust-lang.org has no asset for this platform"))?;
        let resolved_version = if matches!(version, "stable" | "beta" | "nightly") {
            resolve_channel(ctx.http, version).await?
        } else {
            version.to_string()
        };
        let asset = format!("rust-{resolved_version}-{triple}.tar.gz");
        let asset_url = format!("{DIST_BASE}/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = ctx
            .http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = sha_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty sha256 sidecar for {asset}"))?
            .to_string();

        let bytes = common::download_and_verify(
            ctx.http,
            &asset_url,
            &expected,
            ctx.progress,
            &format!("rust {resolved_version}"),
        )
        .await?;

        let store_dir = common::stage_to_store(&paths, &bytes, &expected, &asset)?;

        let top = find_unified_top(&store_dir)?;
        let merged_root = store_dir.join("merged");
        anyv_core::paths::ensure_dir(&merged_root)?;
        merge_components(&top, &merged_root)?;

        common::finalize_install(&merged_root, &install_dir)?;

        Ok(InstallReport {
            version: resolved_version,
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        common::uninstall_version("rust", version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        common::list_installed_versions("rust")
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
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
             '{name}' has no qusp-managed install path."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = common::qusp_paths()?;
        let root = common::lang_root(&paths, "rust", version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("RUSTUP_TOOLCHAIN".into(), version.to_string());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("cargo"),
            FarmBinary::unversioned("rustc"),
            FarmBinary::unversioned("rustdoc"),
            FarmBinary::unversioned("rustfmt"),
            FarmBinary::unversioned("clippy-driver"),
            FarmBinary::unversioned("rust-analyzer"),
            FarmBinary::unversioned("rust-gdb"),
            FarmBinary::unversioned("rust-lldb"),
        ]
    }
}

// ─── Channel resolution ─────────────────────────────────────────────

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
                if let Some(v) = raw.split_whitespace().next() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

// ─── Component merge ────────────────────────────────────────────────

fn find_unified_top(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let p = e.path();
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
            continue;
        }
        copy_tree(&comp_dir, dest)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let from = e.path();
        let name = e.file_name();
        if name == "manifest.in" {
            continue;
        }
        let to = dest.join(&name);
        let ft = e.file_type()?;
        if ft.is_dir() {
            anyv_core::paths::ensure_dir(&to)?;
            copy_tree(&from, &to)?;
        } else if ft.is_symlink() {
            copy_symlink(&from, &to)?;
        } else if !to.exists() {
            copy_file_with_perms(&from, &to)?;
        }
    }
    Ok(())
}

fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    let target = std::fs::read_link(from)?;
    if to.exists() || to.is_symlink() {
        let _ = std::fs::remove_file(to);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, to)?;
    #[cfg(windows)]
    {
        let _ = target;
        std::fs::copy(from, to)?;
    }
    Ok(())
}

fn copy_file_with_perms(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)?;
    #[cfg(unix)]
    {
        let perms = std::fs::metadata(from)?.permissions();
        std::fs::set_permissions(to, perms)?;
    }
    Ok(())
}

// ─── Toolchain file parsing ─────────────────────────────────────────

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
