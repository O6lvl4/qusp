//! Julia backend.
//!
//! Toolchain comes from `julialang-s3.julialang.org/bin/versions.json`
//! — a single document that catalogs every published Julia release
//! with `{ os, arch, kind, triplet, url, sha256, size, ... }` per
//! platform-specific file. One fetch, one parse, sha256 inline.
//!
//! Tools: empty by design. Pkg.jl is Julia's package manager and is
//! invoked from inside the REPL — qusp doesn't shadow it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct JuliaBackend;

const VERSIONS_URL: &str = "https://julialang-s3.julialang.org/bin/versions.json";

/// julialang-s3 names macOS as `"mac"` and Windows as `"winnt"`.
/// Architectures use plain `"x86_64"`/`"aarch64"`, matching std::env::consts::ARCH.
fn host_os_arch() -> Option<(&'static str, &'static str)> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("mac", "aarch64"),
        ("macos", "x86_64") => ("mac", "x86_64"),
        ("linux", "x86_64") => ("linux", "x86_64"),
        ("linux", "aarch64") => ("linux", "aarch64"),
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn julia_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("julia").join(version)
}

#[derive(Debug, Deserialize)]
pub(crate) struct JuliaFile {
    pub(crate) os: String,
    pub(crate) arch: String,
    #[serde(default)]
    pub(crate) extension: String,
    #[serde(default)]
    pub(crate) kind: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JuliaVersion {
    pub(crate) files: Vec<JuliaFile>,
}

#[async_trait]
impl Backend for JuliaBackend {
    fn id(&self) -> &'static str {
        "julia"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".julia-version", "Project.toml"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".julia-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".julia-version".into(),
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
        let install_dir = julia_root(&paths, version);
        if install_dir.join("bin").join("julia").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }
        let (os, arch) =
            host_os_arch().ok_or_else(|| anyhow!("julialang-s3 has no asset for this platform"))?;

        let body = http
            .get_text(VERSIONS_URL)
            .await
            .with_context(|| format!("fetch {VERSIONS_URL}"))?;
        let file = pick_julia_file(&body, version, os, arch)?
            .ok_or_else(|| anyhow!("no Julia archive for {version} on {os}/{arch}"))?;

        let mut task = progress.start(&format!("downloading julia {version}"), None);
        let bytes = http
            .get_bytes_streaming(&file.url, task.as_mut())
            .await
            .with_context(|| format!("download {}", file.url))?;
        task.finish(format!("downloaded julia {version}"));
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !file.sha256.eq_ignore_ascii_case(&actual) {
            bail!(
                "sha256 mismatch for {}: expected {}, got {actual}",
                file.url,
                file.sha256
            );
        }

        let cache_path = paths
            .cache
            .join(format!("julia-{version}-{os}-{arch}.tar.gz"));
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        // Tarball expands to `julia-{version}/{bin/julia, lib/, share/julia/}`.
        let inner = find_julia_top(&store_dir)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        if install_dir.exists() || install_dir.is_symlink() {
            let _ = std::fs::remove_file(&install_dir);
            let _ = std::fs::remove_dir_all(&install_dir);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&inner, &install_dir)
            .with_context(|| format!("symlink {} → {}", install_dir.display(), inner.display()))?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&inner, &install_dir)
            .with_context(|| format!("symlink {} → {}", install_dir.display(), inner.display()))?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = julia_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("julia {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("julia");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let body = http.get_text(VERSIONS_URL).await?;
        Ok(list_julia_versions(&body))
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = julia_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }
}

/// Pure: pick the right archive for `(version, os, arch)` from a
/// `versions.json` body. Returns the file with `kind = "archive"` and
/// `extension = "tar.gz"`.
pub(crate) fn pick_julia_file(
    body: &str,
    version: &str,
    os: &str,
    arch: &str,
) -> Result<Option<JuliaFile>> {
    let root: serde_json::Value =
        serde_json::from_str(body).context("parse Julia versions.json")?;
    let Some(version_entry) = root.get(version) else {
        return Ok(None);
    };
    let v: JuliaVersion =
        serde_json::from_value(version_entry.clone()).context("parse Julia version entry")?;
    Ok(v.files
        .into_iter()
        .find(|f| f.os == os && f.arch == arch && f.kind == "archive" && f.extension == "tar.gz"))
}

/// Pure: stable versions only (no `-rc`, no `-beta`), newest first.
pub(crate) fn list_julia_versions(body: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(obj) = root.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<String> = obj.keys().filter(|k| !k.contains('-')).cloned().collect();
    out.sort_by(|a, b| version_cmp(b, a));
    out
}

fn find_julia_top(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let p = e.path();
            if p.join("bin").join("julia").is_file() {
                return Ok(p);
            }
        }
    }
    bail!(
        "no `bin/julia` inside extracted archive at {}",
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

    const SAMPLE: &str = r#"{
      "1.10.4": {
        "files": [
          {
            "os": "mac", "arch": "aarch64", "kind": "archive", "extension": "tar.gz",
            "triplet": "aarch64-apple-darwin",
            "url": "https://x.test/1.10.4/macos-aarch64.tar.gz",
            "sha256": "aaa111", "size": 100
          },
          {
            "os": "linux", "arch": "x86_64", "kind": "archive", "extension": "tar.gz",
            "triplet": "x86_64-linux-gnu",
            "url": "https://x.test/1.10.4/linux-x86_64.tar.gz",
            "sha256": "bbb222", "size": 100
          },
          {
            "os": "mac", "arch": "aarch64", "kind": "installer", "extension": "dmg",
            "triplet": "aarch64-apple-darwin",
            "url": "https://x.test/1.10.4/macos.dmg",
            "sha256": "ccc333", "size": 100
          }
        ]
      },
      "1.10.0-rc1": {
        "files": [
          {
            "os": "mac", "arch": "aarch64", "kind": "archive", "extension": "tar.gz",
            "triplet": "aarch64-apple-darwin",
            "url": "https://x.test/rc/macos-aarch64.tar.gz",
            "sha256": "rrr", "size": 100
          }
        ]
      },
      "1.9.4": {
        "files": [
          {
            "os": "linux", "arch": "x86_64", "kind": "archive", "extension": "tar.gz",
            "triplet": "x86_64-linux-gnu",
            "url": "https://x.test/1.9.4/linux-x86_64.tar.gz",
            "sha256": "ddd444", "size": 100
          }
        ]
      }
    }"#;

    #[test]
    fn picks_archive_kind_skipping_installer() {
        let f = pick_julia_file(SAMPLE, "1.10.4", "mac", "aarch64")
            .unwrap()
            .unwrap();
        assert_eq!(f.kind, "archive");
        assert_eq!(f.sha256, "aaa111");
        assert!(f.url.ends_with("macos-aarch64.tar.gz"));
    }

    #[test]
    fn returns_none_when_arch_missing() {
        // 1.9.4 only has linux x86_64 in our fixture.
        assert!(pick_julia_file(SAMPLE, "1.9.4", "mac", "aarch64")
            .unwrap()
            .is_none());
    }

    #[test]
    fn returns_none_when_version_missing() {
        assert!(pick_julia_file(SAMPLE, "9.9.9", "mac", "aarch64")
            .unwrap()
            .is_none());
    }

    #[test]
    fn list_filters_pre_releases() {
        let v = list_julia_versions(SAMPLE);
        assert_eq!(v, vec!["1.10.4", "1.9.4"]);
    }
}
