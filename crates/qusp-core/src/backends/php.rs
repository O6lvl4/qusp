//! PHP backend — static binaries from php-build-standalone.
//!
//! Pre-built static PHP binaries from `O6lvl4/php-build-standalone`,
//! sha256-verified against per-asset `.sha256` sidecar. The binaries
//! include 30 extensions (curl, mbstring, openssl, pdo_mysql, etc.)
//! covering Laravel, Symfony, and most modern PHP projects.
//!
//! Toolchain only — Composer is a separate download (phar). A future
//! tool registry entry could auto-fetch `composer.phar`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct PhpBackend;

const REPO: &str = "O6lvl4/php-build-standalone";

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn php_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("php").join(version)
}

#[async_trait]
impl Backend for PhpBackend {
    fn id(&self) -> &'static str {
        "php"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["composer.json", ".php-version"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".php-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".php-version".into(),
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
        _opts: &InstallOpts,
        http: &dyn crate::effects::HttpFetcher,
        progress: &dyn crate::effects::ProgressReporter,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = php_root(&paths, version);
        if install_dir.join("bin").join("php").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;
        let triple = target_triple()
            .ok_or_else(|| anyhow!("php-build-standalone has no binary for this platform"))?;
        let tag = format!("php-{version}");
        let asset = format!("php-{version}-{triple}.tar.gz");
        let sha_asset = format!("{asset}.sha256");
        let base = format!("https://github.com/{REPO}/releases/download/{tag}");
        let asset_url = format!("{base}/{asset}");
        let sha_url = format!("{base}/{sha_asset}");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = sha_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty .sha256 for {asset}"))?
            .to_string();

        let mut task = progress.start(&format!("downloading php {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded php {version}"));

        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
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

        // Archive layout: php-{ver}-{triple}/php
        let inner = find_php_dir(&store_dir)?;
        let php_bin = inner.join("php");
        if !php_bin.is_file() {
            bail!(
                "extracted archive did not contain `php` at {}",
                php_bin.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&php_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&php_bin, perms)?;
        }

        // Create bin/ subdir for consistent layout.
        let bin_dir = inner.join("bin");
        if !bin_dir.exists() {
            anyv_core::paths::ensure_dir(&bin_dir)?;
            let bin_link = bin_dir.join("php");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&php_bin, &bin_link)?;
            #[cfg(windows)]
            std::fs::copy(&php_bin, &bin_link)?;
        }

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&inner, &install_dir)
            .with_context(|| format!("symlink {} → {}", install_dir.display(), inner.display()))?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = php_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("php {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("php");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".qusp-lock") {
                continue;
            }
            out.push(name);
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        #[derive(serde::Deserialize)]
        struct R {
            tag_name: String,
        }
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse php-build-standalone release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter_map(|r| r.tag_name.strip_prefix("php-").map(String::from))
            .collect();
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        _http: &dyn crate::effects::HttpFetcher,
        name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "PHP tools are typically installed via Composer. \
             Use `composer global require {name}` or add it to \
             your project's composer.json."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = php_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: std::collections::BTreeMap::new(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![FarmBinary::unversioned("php")]
    }
}

/// Find the extracted directory inside the store (e.g. `php-8.4.20-aarch64-apple-darwin/`).
fn find_php_dir(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("php-") {
                return Ok(e.path());
            }
        }
    }
    bail!("no php-* directory found inside {}", store_dir.display())
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
