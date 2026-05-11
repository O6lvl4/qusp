//! Bun backend — native installer, toolchain only.
//!
//! `bun` ships as a single self-contained binary from oven-sh/bun GitHub
//! releases. qusp downloads `bun-<triple>.zip`, verifies it against the
//! release's `SHASUMS256.txt`, extracts the binary, and symlinks
//! `versions/bun/<v>` at it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;

use super::common;
use crate::backend::*;

pub struct BunBackend;

const REPO: &str = "oven-sh/bun";

fn target_triple() -> Option<&'static str> {
    Some(match common::os_arch() {
        ("macos", "aarch64") => "darwin-aarch64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => return None,
    })
}

fn strip_v(v: &str) -> &str {
    let v = v.strip_prefix("bun-").unwrap_or(v);
    v.strip_prefix('v').unwrap_or(v)
}

#[derive(Deserialize, Debug)]
struct GhRelease {
    tag_name: String,
}

#[async_trait]
impl Backend for BunBackend {
    fn id(&self) -> &'static str {
        "bun"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["bun.lockb", "bun.lock", ".bun-version", "package.json"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".bun-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".bun-version".into(),
                        origin: f,
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
        let v_strip = strip_v(version);
        let install_dir = common::lang_root(&paths, "bun", v_strip);

        if let Some(report) = common::check_already_installed(&install_dir, "bin/bun", v_strip) {
            return Ok(report);
        }
        let _guard = common::acquire_install_lock(&install_dir)?;

        let triple =
            target_triple().ok_or_else(|| anyhow!("oven-sh/bun has no asset for this platform"))?;
        let tag = format!("bun-v{v_strip}");
        let asset = format!("bun-{triple}.zip");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sums_url = format!("https://github.com/{REPO}/releases/download/{tag}/SHASUMS256.txt");

        let sums_text = ctx
            .http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = crate::backends::node::parse_shasums_line(&sums_text, &asset)
            .ok_or_else(|| anyhow!("no entry for {asset} in SHASUMS256.txt"))?;

        let bytes = common::download_and_verify(
            ctx.http,
            &asset_url,
            &expected,
            ctx.progress,
            &format!("bun {version}"),
        )
        .await?;

        let store_dir = common::stage_to_store(&paths, &bytes, &expected, &asset)?;

        // The zip extracts a single `bun-<triple>/` dir containing `bun`.
        let inner = store_dir.join(format!("bun-{triple}"));
        let bun_bin = if inner.join("bun").is_file() {
            inner.join("bun")
        } else if store_dir.join("bun").is_file() {
            store_dir.join("bun")
        } else {
            bail!(
                "extracted Bun archive did not contain a `bun` binary at {} or {}/bun",
                inner.display(),
                store_dir.display()
            );
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bun_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&bun_bin, perms)?;
        }
        let bin_dir = store_dir.join("bin");
        anyv_core::paths::ensure_dir(&bin_dir)?;
        let bin_link = bin_dir.join("bun");
        if bin_link.exists() || bin_link.is_symlink() {
            let _ = std::fs::remove_file(&bin_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&bun_bin, &bin_link)?;
        #[cfg(windows)]
        std::fs::copy(&bun_bin, &bin_link)?;
        let bunx_link = bin_dir.join("bunx");
        if bunx_link.exists() || bunx_link.is_symlink() {
            let _ = std::fs::remove_file(&bunx_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&bun_bin, &bunx_link)?;
        #[cfg(windows)]
        std::fs::copy(&bun_bin, &bunx_link)?;

        common::finalize_install(&store_dir, &install_dir)?;

        Ok(InstallReport {
            version: v_strip.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        common::uninstall_version("bun", version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        common::list_installed_versions("bun")
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<GhRelease> =
            serde_json::from_str(&body).context("parse oven-sh/bun release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .map(|r| strip_v(&r.tag_name).to_string())
            .filter(|v| !v.starts_with("canary"))
            .collect();
        out.sort_by(|a, b| common::version_cmp(b, a));
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        _http: &dyn crate::effects::HttpFetcher,
        name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "Bun doesn't have a qusp-managed tool registry. Use Bun's own `bun install` \
             for npm packages, or pin Node-side CLIs under `[node.tools]`. \
             '{name}' has no qusp install path on the bun backend."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = common::qusp_paths()?;
        let root = common::lang_root(&paths, "bun", version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("bun"),
            FarmBinary::unversioned("bunx"),
        ]
    }
}
