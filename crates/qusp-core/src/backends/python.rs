//! Python backend — native installer.
//!
//! Downloads [python-build-standalone](https://github.com/astral-sh/python-build-standalone)
//! tarballs directly from GitHub releases. Verifies sha256. Extracts via
//! `anyv_core::extract`. **No `uv` runtime dependency.** uv is recommended
//! for actual Python project workflows (deps, venvs, scripts) but qusp
//! manages the interpreter itself end-to-end.
//!
//! Astral publishes python-build-standalone tarballs as a public artifact;
//! qusp consumes them the same way `gv` consumes go.dev tarballs and `rv`
//! consumes ruby-source archives — with an open-source license, sha256
//! verification, and clear attribution.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct PythonBackend;

const REPO: &str = "astral-sh/python-build-standalone";

#[derive(Deserialize, Debug)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize, Debug)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

/// qusp owns Python under its own paths since there's no standalone `pv` tool.
fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn python_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("python").join(version)
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("qusp-python/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

#[async_trait]
impl Backend for PythonBackend {
    fn id(&self) -> &'static str {
        "python"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["pyproject.toml", ".python-version"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".python-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".python-version".into(),
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
        _qusp_paths: &AnyvPaths,
        version: &str,
        _opts: &InstallOpts,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = python_root(&paths, version);
        if install_dir.join("bin").join("python3").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        let triple = target_triple()
            .ok_or_else(|| anyhow!("python-build-standalone has no asset for this platform"))?;
        let client = http_client()?;

        // Walk the most-recent releases until we find one with the requested
        // version (the asset filename's prefix, before `+<build_tag>`).
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
        let releases: Vec<GhRelease> = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("parse python-build-standalone release index")?;

        let asset_prefix = format!("cpython-{version}+");
        let asset_suffix = format!("-{triple}-install_only_stripped.tar.gz");
        let asset = releases
            .iter()
            .flat_map(|r| r.assets.iter())
            .find(|a| a.name.starts_with(&asset_prefix) && a.name.ends_with(&asset_suffix))
            .ok_or_else(|| {
                anyhow!(
                    "no python-build-standalone asset found for {version} on {triple} \
                 (looked at the {n} most recent releases)",
                    n = releases.len(),
                )
            })?;

        // python-build-standalone publishes a single `SHA256SUMS` file per
        // release (one line per asset: `<hash>  <filename>`). Find the
        // release this asset belongs to so we can grab the matching SUMS.
        let owning_release = releases
            .iter()
            .find(|r| r.assets.iter().any(|a| a.name == asset.name))
            .ok_or_else(|| anyhow!("internal: lost track of asset's release"))?;
        let sums_asset = owning_release
            .assets
            .iter()
            .find(|a| a.name == "SHA256SUMS")
            .ok_or_else(|| {
                anyhow!(
                    "release {} has no SHA256SUMS file; refusing to install without verification",
                    owning_release.tag_name
                )
            })?;
        let sums_text = client
            .get(&sums_asset.browser_download_url)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("fetch {}", sums_asset.browser_download_url))?
            .text()
            .await?;
        let expected = sums_text
            .lines()
            .find_map(|l| {
                let mut parts = l.split_whitespace();
                let hash = parts.next()?;
                let filename = parts.next()?;
                if filename == asset.name {
                    Some(hash.to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("no entry for {} in SHA256SUMS", asset.name))?;

        let bytes = client
            .get(&asset.browser_download_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .with_context(|| format!("download {}", asset.name))?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if expected != actual {
            bail!(
                "sha256 mismatch for {}: expected {expected}, got {actual}",
                asset.name
            );
        }

        // Stage in cache, extract into a content-addressed dir, then symlink
        // versions/python/<version> at it (handled by python_root path).
        let cache_path = paths.cache.join(&asset.name);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)
            .with_context(|| format!("write {}", cache_path.display()))?;

        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;

        // The tarball expands to a top-level `python/` dir.
        let inner = store_dir.join("python");
        let real_install = if inner.is_dir() {
            inner
        } else {
            store_dir.clone()
        };

        // Place the version-named symlink. Wipe any prior one first.
        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        if install_dir.exists() || install_dir.is_symlink() {
            let _ = std::fs::remove_file(&install_dir);
            let _ = std::fs::remove_dir_all(&install_dir);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_install, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_install.display()
            )
        })?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_install, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_install.display()
            )
        })?;

        let _ = std::fs::remove_file(&cache_path);
        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = python_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("python {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("python");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let n = e.file_name().to_string_lossy().to_string();
            if n.chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                out.push(n);
            }
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>> {
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=5");
        let releases: Vec<GhRelease> = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "qusp")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for r in &releases {
            for a in &r.assets {
                if let Some(rest) = a.name.strip_prefix("cpython-") {
                    if let Some(plus) = rest.find('+') {
                        seen.insert(rest[..plus].to_string());
                    }
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        _: &reqwest::Client,
        _name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "Python tool management is delegated to uv — install per-tool with `uv tool install <name>` \
             or run ad-hoc with `uvx <name>`. qusp v0.2.0 will route `qusp tool add` to uv \
             for users who prefer a single CLI."
        )
    }

    async fn install_tool(
        &self,
        _: &AnyvPaths,
        _toolchain_version: &str,
        _resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        bail!("Python tool routing through uv arrives in v0.2.0.")
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = python_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}
