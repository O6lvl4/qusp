//! Crystal backend.
//!
//! Toolchain comes from the `crystal-lang/crystal` GitHub releases.
//! Crystal doesn't publish a sha256 sidecar, but GitHub's API exposes
//! `asset.digest = "sha256:HEX"` for every release asset — qusp uses
//! that as the verification source.
//!
//! Tools: empty by design. Shards (Crystal's package manager) is
//! per-project, run from `shard.yml` — qusp doesn't shadow it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct CrystalBackend;

const REPO: &str = "crystal-lang/crystal";

/// Crystal asset slug. macOS is universal (no arch split); Linux splits
/// by arch. Windows is unsupported by upstream so qusp skips.
fn host_slug() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => "darwin-universal",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn crystal_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("crystal").join(version)
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    #[allow(dead_code)]
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    /// `"sha256:abc…"` form. Published by GitHub for every release asset.
    #[serde(default)]
    digest: String,
}

#[async_trait]
impl Backend for CrystalBackend {
    fn id(&self) -> &'static str {
        "crystal"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".crystal-version", "shard.yml"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".crystal-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".crystal-version".into(),
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
        let install_dir = crystal_root(&paths, version);
        if install_dir.join("bin").join("crystal").exists() {
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
        let slug = host_slug()
            .ok_or_else(|| anyhow!("crystal-lang/crystal has no asset for this platform"))?;

        // GitHub release JSON has `assets[*].digest = "sha256:HEX"`.
        let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{version}");
        let body = http.get_text_authenticated(&url).await.with_context(|| {
            format!("fetch crystal release {version} (is the tag {version} valid?)")
        })?;
        let release: GhRelease =
            serde_json::from_str(&body).context("parse crystal release JSON")?;

        let asset = pick_crystal_asset(&release.assets, version, slug).ok_or_else(|| {
            anyhow!(
                "no Crystal asset for {version} on {slug} \
                 (looked at {n} assets in the release)",
                n = release.assets.len(),
            )
        })?;
        let expected_sha = parse_sha256_digest(&asset.digest).ok_or_else(|| {
            anyhow!(
                "release asset {} has no sha256 digest from GitHub — refusing to install \
                 without verification",
                asset.name
            )
        })?;

        let mut task = progress.start(&format!("downloading crystal {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset.browser_download_url, task.as_mut())
            .await
            .with_context(|| format!("download {}", asset.browser_download_url))?;
        task.finish(format!("downloaded crystal {version}"));
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected_sha.eq_ignore_ascii_case(&actual) {
            bail!(
                "sha256 mismatch for {}: expected {expected_sha}, got {actual}",
                asset.name
            );
        }

        let cache_path = paths.cache.join(&asset.name);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        // Tarball expands to `crystal-{version}-{n}/{bin/, share/, src/}`.
        let inner = find_crystal_top(&store_dir)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&inner, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), inner.display())
            })?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = crystal_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("crystal {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("crystal");
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
        #[derive(Deserialize)]
        struct R {
            tag_name: String,
            #[serde(default)]
            prerelease: bool,
        }
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse crystal releases index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| r.tag_name)
            .collect();
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = crystal_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("crystal"),
            FarmBinary::unversioned("shards"),
        ]
    }
}

/// Pure: among a release's assets, pick the one whose name matches
/// `crystal-{version}-{N}-{slug}.tar.gz`. The N (build number) is
/// usually `-1-` but we don't hardcode it.
fn pick_crystal_asset<'a>(assets: &'a [GhAsset], version: &str, slug: &str) -> Option<&'a GhAsset> {
    let prefix = format!("crystal-{version}-");
    let suffix = format!("-{slug}.tar.gz");
    assets
        .iter()
        .find(|a| a.name.starts_with(&prefix) && a.name.ends_with(&suffix))
}

/// Pure: GitHub publishes asset digests as `"sha256:HEX"`. Strip the
/// prefix and return the hex bytes (or None if format is unexpected).
fn parse_sha256_digest(s: &str) -> Option<String> {
    s.strip_prefix("sha256:").map(|h| h.trim().to_string())
}

fn find_crystal_top(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let p = e.path();
            if p.join("bin").join("crystal").is_file() {
                return Ok(p);
            }
        }
    }
    bail!(
        "no `bin/crystal` inside extracted archive at {}",
        store_dir.display()
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, sha: &str) -> GhAsset {
        GhAsset {
            name: name.to_string(),
            browser_download_url: format!("https://x.test/{name}"),
            digest: format!("sha256:{sha}"),
        }
    }

    #[test]
    fn picks_darwin_universal_for_macos() {
        let assets = vec![
            asset("crystal-1.20.0-1-darwin-universal.tar.gz", "aaa"),
            asset("crystal-1.20.0-1-linux-x86_64.tar.gz", "bbb"),
            asset("crystal-1.20.0-1.universal.pkg", "ccc"),
        ];
        let a = pick_crystal_asset(&assets, "1.20.0", "darwin-universal").unwrap();
        assert!(a.name.contains("darwin-universal"));
    }

    #[test]
    fn returns_none_when_slug_missing() {
        let assets = vec![asset("crystal-1.20.0-1-linux-x86_64.tar.gz", "bbb")];
        assert!(pick_crystal_asset(&assets, "1.20.0", "darwin-universal").is_none());
    }

    #[test]
    fn parses_github_digest_format() {
        assert_eq!(
            parse_sha256_digest("sha256:abc123").as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn rejects_unsupported_digest_form() {
        assert!(parse_sha256_digest("md5:xxx").is_none());
        assert!(parse_sha256_digest("abc").is_none());
    }

    #[test]
    fn skips_bundled_variants() {
        // -bundled.tar.gz should NOT match plain darwin-universal slug.
        let assets = vec![
            asset("crystal-1.20.0-1-linux-x86_64.tar.gz", "aaa"),
            asset("crystal-1.20.0-1-linux-x86_64-bundled.tar.gz", "bbb"),
        ];
        let a = pick_crystal_asset(&assets, "1.20.0", "linux-x86_64").unwrap();
        assert_eq!(a.digest, "sha256:aaa");
        assert!(!a.name.contains("bundled"));
    }
}
