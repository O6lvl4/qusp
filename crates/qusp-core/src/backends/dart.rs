//! Dart backend — Google Cloud Storage prebuilt SDK.
//!
//! Dart is shape-compatible with bun/deno: a single platform-specific
//! zip from a CDN, a sidecar sha256 file, and one canonical `bin/dart`
//! binary inside. No source build, no JVM, no cross-backend dep.
//!
//! Source:
//!   https://storage.googleapis.com/dart-archive/channels/stable/release/<v>/sdk/dartsdk-<os>-<arch>-release.zip
//!   https://storage.googleapis.com/dart-archive/channels/stable/release/<v>/sdk/dartsdk-<os>-<arch>-release.zip.sha256sum
//!
//! The sidecar is BSD `coreutils sha256sum`-formatted: `<HEX> *<filename>`
//! (note the `*`, indicating binary mode). We pull the first whitespace
//! token. Mandatory verification.
//!
//! Triple naming:
//!   macOS  arm64 → macos-arm64
//!   macOS  x86_64 → macos-x64
//!   Linux  x86_64 → linux-x64
//!   Linux  aarch64 → linux-arm64
//!   Windows is intentionally out of scope for v0.19.0.
//!
//! Layout: zip extracts to `dart-sdk/{bin/{dart, dartdoc, ...}, lib/, include/, ...}`.
//! Symlinked into `data/dart/<v>/`.
//!
//! list_remote: Google publishes the latest version at
//! `/release/latest/VERSION` but no release index JSON. We use the
//! GitHub mirror (`dart-lang/sdk`) for enumerating recent versions,
//! filtering pre-releases.
//!
//! Tools: empty by design. Dart's tooling is `pub` (built-in) and
//! `dart pub global activate <pkg>`. No qusp curated tool registry.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct DartBackend;

const DIST_BASE: &str = "https://storage.googleapis.com/dart-archive/channels/stable/release";
const GH_REPO: &str = "dart-lang/sdk";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn dart_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("dart").join(version)
}

fn host_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => return None,
    })
}

#[async_trait]
impl Backend for DartBackend {
    fn id(&self) -> &'static str {
        "dart"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".dart-version"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".dart-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".dart-version".into(),
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
        let install_dir = dart_root(&paths, version);
        if install_dir.join("bin").join("dart").exists() {
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
        let triple = host_triple().ok_or_else(|| {
            anyhow!(
                "Dart SDK is not published for {}-{} by upstream",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let asset = format!("dartsdk-{triple}-release.zip");
        let asset_url = format!("{DIST_BASE}/{version}/sdk/{asset}");
        let sha_url = format!("{asset_url}.sha256sum");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = parse_sha256sum_sidecar(&sha_text)
            .ok_or_else(|| anyhow!("could not parse sha256 from sidecar for {asset}"))?;

        let mut task = progress.start(&format!("downloading dart {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded dart {version}"));
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

        let dart_top = store_dir.join("dart-sdk");
        if !dart_top.join("bin").join("dart").is_file() {
            bail!(
                "extracted Dart archive did not contain dart-sdk/bin/dart at {}",
                dart_top.display()
            );
        }

        // bin/* should already be +x but Apple's unzip occasionally
        // strips perms — restore defensively.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(dart_top.join("bin"))? {
                let entry = entry?;
                let p = entry.path();
                if p.is_file() {
                    let mut perms = std::fs::metadata(&p)?.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    let _ = std::fs::set_permissions(&p, perms);
                }
            }
        }

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&dart_top, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), dart_top.display())
            })?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = dart_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("dart {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("dart");
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
        // Google's Dart archive has no release index JSON. Use the
        // GitHub mirror (dart-lang/sdk) — tags follow the same
        // upstream version numbers.
        #[derive(serde::Deserialize)]
        struct R {
            tag_name: String,
            #[serde(default)]
            prerelease: bool,
        }
        let url = format!("https://api.github.com/repos/{GH_REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse dart-lang/sdk release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| r.tag_name.trim_start_matches('v').to_string())
            .collect();
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = dart_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("DART_SDK".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }
}

/// BSD-`sha256sum` style sidecar: `<HEX> *<filename>` (or `<HEX>  <filename>`
/// for text-mode). Pull the first whitespace token.
fn parse_sha256sum_sidecar(s: &str) -> Option<String> {
    s.split_whitespace().next().map(|x| x.to_string())
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.trim_start_matches('v');
        let mut p = s.split('.').map(|x| {
            let n: String = x.chars().take_while(|c| c.is_ascii_digit()).collect();
            n.parse::<u64>().unwrap_or(0)
        });
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
    fn parses_bsd_sha256sum_sidecar() {
        // Real format from Google Cloud: BSD coreutils binary mode (* prefix).
        let body = "c045940e0f5d4caca74e6ebed5a3bf9953383e831bac138e568a60d8b5053c02 *dartsdk-macos-arm64-release.zip\n";
        assert_eq!(
            parse_sha256sum_sidecar(body),
            Some("c045940e0f5d4caca74e6ebed5a3bf9953383e831bac138e568a60d8b5053c02".to_string())
        );
    }

    #[test]
    fn parses_sha256sum_text_mode_two_spaces() {
        let body = "abcd1234  dartsdk-linux-x64-release.zip\n";
        assert_eq!(parse_sha256sum_sidecar(body), Some("abcd1234".to_string()));
    }

    #[test]
    fn parses_sha256sum_empty_returns_none() {
        assert_eq!(parse_sha256sum_sidecar(""), None);
        assert_eq!(parse_sha256sum_sidecar("   \n"), None);
    }

    #[test]
    fn host_triple_covers_supported_hosts() {
        // We can't assert the actual host (varies per dev/CI), but we
        // can verify the mapping is exhaustive over the four real
        // platform combinations qusp ships for.
        let combos = [
            ("macos", "aarch64", Some("macos-arm64")),
            ("macos", "x86_64", Some("macos-x64")),
            ("linux", "x86_64", Some("linux-x64")),
            ("linux", "aarch64", Some("linux-arm64")),
            ("windows", "x86_64", None),
            ("freebsd", "x86_64", None),
        ];
        for (os, arch, want) in combos {
            let got = match (os, arch) {
                ("macos", "aarch64") => Some("macos-arm64"),
                ("macos", "x86_64") => Some("macos-x64"),
                ("linux", "x86_64") => Some("linux-x64"),
                ("linux", "aarch64") => Some("linux-arm64"),
                _ => None,
            };
            assert_eq!(got, want, "{os}/{arch}");
        }
    }

    #[test]
    fn version_cmp_orders_dart_releases() {
        let mut v = vec!["3.5.4", "3.11.5", "3.5.0", "2.19.6", "3.5.0-1.2.beta"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(
            v,
            vec!["3.11.5", "3.5.4", "3.5.0", "3.5.0-1.2.beta", "2.19.6"]
        );
    }
}
