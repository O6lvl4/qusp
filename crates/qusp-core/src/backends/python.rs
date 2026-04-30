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
pub(crate) struct GhRelease {
    pub(crate) assets: Vec<GhAsset>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct GhAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
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
        ctx: &crate::backend::InstallCtx<'_>,
    ) -> Result<InstallReport> {
        let http = ctx.http;
        let progress = ctx.progress;

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

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;

        let triple = target_triple()
            .ok_or_else(|| anyhow!("python-build-standalone has no asset for this platform"))?;

        // Walk the most-recent releases until we find one with the requested
        // version (the asset filename's prefix, before `+<build_tag>`).
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
        let releases_text = http
            .get_text_authenticated(&url)
            .await
            .with_context(|| format!("fetch {url}"))?;
        let releases: Vec<GhRelease> = serde_json::from_str(&releases_text)
            .context("parse python-build-standalone release index")?;

        let asset = pick_pbs_asset(&releases, version, triple).ok_or_else(|| {
            anyhow!(
                "no python-build-standalone asset found for {version} on {triple} \
                 (looked at the {n} most recent releases). Try a different patch like \
                 `python = \"{}.0\"` and qusp will auto-select the latest patch.",
                python_minor_prefix(version),
                n = releases.len(),
            )
        })?;

        let (sums_url, asset_url) =
            sums_and_asset_urls(&releases, &asset.name).ok_or_else(|| {
                anyhow!(
                    "release containing {} has no SHA256SUMS file; refusing to install without verification",
                    asset.name
                )
            })?;
        let sums_text = http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = parse_sums_line(&sums_text, &asset.name)
            .ok_or_else(|| anyhow!("no entry for {} in SHA256SUMS", asset.name))?;

        let mut task = progress.start(&format!("downloading {}", asset.name), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {}", asset.name))?;
        task.finish(format!("downloaded {}", asset.name));

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
        crate::effects::atomic_symlink_swap(&real_install, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_install.display()
            )
        })?;

        let _ = std::fs::remove_file(&cache_path);
        // Report the resolved version (e.g. `3.13.5+20260414`) — what
        // PBS actually shipped — alongside the user-pinned install dir
        // (still keyed by the original `version` arg so the lock and
        // future `build_run_env` calls line up).
        let resolved_version = asset
            .name
            .strip_prefix("cpython-")
            .and_then(|s| s.split('-').next())
            .unwrap_or(version)
            .to_string();
        Ok(InstallReport {
            version: resolved_version,
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

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=5");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<GhRelease> =
            serde_json::from_str(&body).context("parse PBS release index")?;
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
        _http: &dyn crate::effects::HttpFetcher,
        _name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "Python tool management is delegated to uv — install per-tool with `uv tool install <name>` \
             or run ad-hoc with `uvx <name>`. qusp v0.2.0 will route `qusp tool add` to uv \
             for users who prefer a single CLI."
        )
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

    /// Python is qusp's richest farm exposure: the install ships
    /// versioned `python3.X` / `pip3.X` (always farmed, no conflict
    /// across installs) plus unversioned `python` / `python3` /
    /// `pip` / `pip3` (only farmed when user has globally pinned
    /// this version).
    fn farm_binaries(&self, version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        let mut bins = Vec::new();
        let mm = python_minor_prefix(version);
        if !mm.is_empty() {
            bins.push(FarmBinary::versioned(format!("python{mm}")));
            bins.push(FarmBinary::versioned(format!("pip{mm}")));
        }
        bins.extend([
            FarmBinary::unversioned("python"),
            FarmBinary::unversioned("python3"),
            FarmBinary::unversioned("pip"),
            FarmBinary::unversioned("pip3"),
        ]);
        bins
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

/// `"3.13.0"` → `"3.13"`. `"3.13"` → `"3.13"`.
fn python_minor_prefix(v: &str) -> String {
    let mut it = v.split('.');
    let major = it.next().unwrap_or("");
    let minor = it.next().unwrap_or("");
    if major.is_empty() || minor.is_empty() {
        v.to_string()
    } else {
        format!("{major}.{minor}")
    }
}

/// Pure: pick the best PBS asset for a given version + triple. Tries
/// exact match first, falls back to latest patch sharing the
/// `<major>.<minor>` prefix.
pub(crate) fn pick_pbs_asset<'a>(
    releases: &'a [GhRelease],
    version: &str,
    triple: &str,
) -> Option<&'a GhAsset> {
    let asset_suffix = format!("-{triple}-install_only_stripped.tar.gz");
    let exact_prefix = format!("cpython-{version}+");
    if let Some(a) = releases
        .iter()
        .flat_map(|r| r.assets.iter())
        .find(|a| a.name.starts_with(&exact_prefix) && a.name.ends_with(&asset_suffix))
    {
        return Some(a);
    }
    let minor = python_minor_prefix(version);
    let fuzzy_prefix = format!("cpython-{minor}.");
    releases
        .iter()
        .flat_map(|r| r.assets.iter())
        .filter(|a| a.name.starts_with(&fuzzy_prefix) && a.name.ends_with(&asset_suffix))
        .max_by(|a, b| compare_pbs_asset_versions(&a.name, &b.name))
}

/// Pure: given the release index and an asset name, return
/// `(sums_url, asset_url)`.
pub(crate) fn sums_and_asset_urls(
    releases: &[GhRelease],
    asset_name: &str,
) -> Option<(String, String)> {
    let owning = releases
        .iter()
        .find(|r| r.assets.iter().any(|a| a.name == asset_name))?;
    let asset = owning.assets.iter().find(|a| a.name == asset_name)?;
    let sums = owning.assets.iter().find(|a| a.name == "SHA256SUMS")?;
    Some((
        sums.browser_download_url.clone(),
        asset.browser_download_url.clone(),
    ))
}

/// Pure: parse a `SHA256SUMS` body and return the hash for the named
/// asset. Lines look like `<hex>  <filename>`.
pub(crate) fn parse_sums_line(body: &str, asset_name: &str) -> Option<String> {
    body.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let hash = parts.next()?;
        let filename = parts.next()?;
        if filename == asset_name {
            Some(hash.to_string())
        } else {
            None
        }
    })
}

/// Compare two PBS asset filenames by the embedded patch+date. Higher
/// patch wins; ties broken by build tag (date stamp).
fn compare_pbs_asset_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(name: &str) -> (u64, u64, u64, u64) {
        // Filename: cpython-3.13.5+20260414-x86_64-apple-darwin-...
        let rest = name.strip_prefix("cpython-").unwrap_or(name);
        let (ver, after_plus) = rest.split_once('+').unwrap_or((rest, ""));
        let mut vp = ver.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        let major = vp.next().unwrap_or(0);
        let minor = vp.next().unwrap_or(0);
        let patch = vp.next().unwrap_or(0);
        let build = after_plus
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        (major, minor, patch, build)
    }
    key(a).cmp(&key(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(_tag: &str, asset_names: &[&str]) -> GhRelease {
        GhRelease {
            assets: asset_names
                .iter()
                .map(|n| GhAsset {
                    name: (*n).to_string(),
                    browser_download_url: format!("https://example.test/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn picks_exact_version_when_present() {
        let rels = vec![release(
            "20260414",
            &[
                "cpython-3.13.0+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
                "cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
                "SHA256SUMS",
            ],
        )];
        let pick = pick_pbs_asset(&rels, "3.13.0", "x86_64-apple-darwin").unwrap();
        assert!(pick.name.starts_with("cpython-3.13.0+"));
    }

    #[test]
    fn falls_back_to_latest_patch_within_minor() {
        // PBS dropped 3.13.0 but still ships 3.13.13.
        let rels = vec![release(
            "20260414",
            &[
                "cpython-3.13.5+20260301-x86_64-apple-darwin-install_only_stripped.tar.gz",
                "cpython-3.13.13+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
                "SHA256SUMS",
            ],
        )];
        let pick = pick_pbs_asset(&rels, "3.13.0", "x86_64-apple-darwin").unwrap();
        assert!(
            pick.name.starts_with("cpython-3.13.13+"),
            "should pick the latest 3.13.x, got {}",
            pick.name
        );
    }

    #[test]
    fn fuzzy_match_picks_higher_build_tag_on_patch_tie() {
        let rels = vec![
            release(
                "20260301",
                &["cpython-3.13.5+20260301-x86_64-apple-darwin-install_only_stripped.tar.gz"],
            ),
            release(
                "20260414",
                &["cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz"],
            ),
        ];
        let pick = pick_pbs_asset(&rels, "3.13.0", "x86_64-apple-darwin").unwrap();
        assert!(
            pick.name.contains("3.13.5+20260414"),
            "should pick newer build tag, got {}",
            pick.name
        );
    }

    #[test]
    fn picks_none_when_no_matching_minor() {
        let rels = vec![release(
            "20260414",
            &["cpython-3.12.10+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz"],
        )];
        assert!(pick_pbs_asset(&rels, "3.13.0", "x86_64-apple-darwin").is_none());
    }

    #[test]
    fn sums_and_asset_urls_pair_correctly() {
        let rels = vec![release(
            "20260414",
            &[
                "cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
                "SHA256SUMS",
            ],
        )];
        let (sums_url, asset_url) = sums_and_asset_urls(
            &rels,
            "cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
        )
        .unwrap();
        assert!(sums_url.ends_with("/SHA256SUMS"));
        assert!(asset_url.contains("3.13.5+20260414"));
    }

    #[test]
    fn parse_sums_line_finds_matching_asset() {
        let body = "\
abc111  cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz
def222  cpython-3.12.10+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz
";
        let hash = parse_sums_line(
            body,
            "cpython-3.13.5+20260414-x86_64-apple-darwin-install_only_stripped.tar.gz",
        )
        .unwrap();
        assert_eq!(hash, "abc111");
    }

    #[test]
    fn parse_sums_line_returns_none_for_missing_asset() {
        let body = "abc111  some-other-file.tar.gz\n";
        assert!(parse_sums_line(body, "cpython-x.y.z.tar.gz").is_none());
    }
}
