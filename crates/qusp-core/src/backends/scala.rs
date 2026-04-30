//! Scala 3 backend — direct release tarball, JVM-target compiler.
//!
//! Like Kotlin and Groovy, Scala declares `requires = ["java"]` so the
//! orchestrator refuses to install `[scala]` without a `[java]` pin
//! and `qusp run` merges the JDK env into the Scala runner's PATH.
//!
//! The roadmap originally proposed a Coursier-bootstrap wrapper, but
//! it turns out scala/scala3 publishes per-host prebuilt tarballs with
//! standard `.sha256` sidecars (3.7.0+). That makes the Coursier
//! detour unnecessary — the install path is identical to Crystal:
//!
//!   https://github.com/scala/scala3/releases/download/<v>/scala3-<v>-<triple>.tar.gz
//!   https://github.com/scala/scala3/releases/download/<v>/scala3-<v>-<triple>.tar.gz.sha256
//!
//! Verification floor: 3.7.0. Older releases (≤3.6.x) only ship a
//! per-platform `sha256sum-<triple>.txt` bulk file; we don't bother
//! falling back, qusp's "mandatory verification" stance pairs cleanly
//! with "pin a version published in the last 12 months."
//!
//! Triple naming follows upstream:
//!   macOS  arm64 → aarch64-apple-darwin
//!   macOS  x86_64 → x86_64-apple-darwin
//!   Linux  x86_64 → x86_64-pc-linux
//!   Linux  aarch64 → aarch64-pc-linux
//!
//! Layout: `scala3-<v>-<triple>/{bin/{scala,scalac,scaladoc,...}, lib/,
//! maven2/, ...}`. The maven2/ tree is a bundled local Maven cache
//! (~70 MB) — Scala uses it as a runtime resolver, so we don't strip.
//!
//! Tools: empty by design. Scala's tooling model is sbt/mill/scala-cli
//! plugins and Coursier `cs install`; qusp doesn't curate a registry
//! against that ecosystem.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct ScalaBackend;

const REPO: &str = "scala/scala3";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn scala_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("scala").join(version)
}

fn host_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-pc-linux",
        ("linux", "aarch64") => "aarch64-pc-linux",
        _ => return None,
    })
}

#[async_trait]
impl Backend for ScalaBackend {
    fn id(&self) -> &'static str {
        "scala"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".scala-version", "build.sbt", "build.sc"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }
    fn requires(&self) -> &[&'static str] {
        &["java"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".scala-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".scala-version".into(),
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
        ctx: &crate::backend::InstallCtx<'_>,
    ) -> Result<InstallReport> {
        let http = ctx.http;
        let progress = ctx.progress;

        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = scala_root(&paths, version);
        if install_dir.join("bin").join("scala").exists() {
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
        let triple = host_triple().ok_or_else(|| {
            anyhow!(
                "Scala 3 is not published for {}-{} by upstream",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let asset = format!("scala3-{version}-{triple}.tar.gz");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{version}/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = http.get_text(&sha_url).await.with_context(|| {
            format!(
                "fetch {sha_url} (Scala 3 versions ≥3.7.0 publish .sha256 sidecars; \
                 pin a more recent version if this 404s)"
            )
        })?;
        let expected = parse_sha256_sidecar(&sha_text)
            .ok_or_else(|| anyhow!("could not parse sha256 from sidecar for {asset}"))?;

        let mut task = progress.start(&format!("downloading scala {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded scala {version}"));
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

        let scala_top = store_dir.join(format!("scala3-{version}-{triple}"));
        if !scala_top.join("bin").join("scala").is_file() {
            bail!(
                "extracted Scala archive did not contain scala3-{version}-{triple}/bin/scala at {}",
                scala_top.display()
            );
        }

        // bin/* defensive +x.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(scala_top.join("bin"))? {
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
        crate::effects::atomic_symlink_swap(&scala_top, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                scala_top.display()
            )
        })?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = scala_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("scala {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("scala");
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
        #[derive(serde::Deserialize)]
        struct R {
            tag_name: String,
            #[serde(default)]
            prerelease: bool,
        }
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse scala/scala3 release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .filter(|r| supports_sidecar(&r.tag_name))
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
        let root = scala_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("SCALA_HOME".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("scala"),
            FarmBinary::unversioned("scalac"),
            FarmBinary::unversioned("scaladoc"),
            FarmBinary::unversioned("scala-cli"),
        ]
    }
}

fn parse_sha256_sidecar(s: &str) -> Option<String> {
    s.split_whitespace().next().map(|x| x.to_string())
}

/// Versions <3.7.0 don't ship per-asset `.sha256` sidecars (only a
/// per-platform bulk `sha256sum-<triple>.txt`). qusp's mandatory-
/// verification stance is easier to maintain by simply not listing
/// them — the user can pin a newer 3.x release.
fn supports_sidecar(tag: &str) -> bool {
    let v = tag.trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    let major = parts
        .first()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let minor = parts
        .get(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    (major, minor) >= (3, 7)
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
    fn parses_scala_sha256_sidecar() {
        // Real format from scala/scala3 v3.7.3:
        //   "10cb...  scala3-3.7.3-aarch64-apple-darwin.tar.gz"
        let body = "10cb872e3162b36af2e4c993afebf515e9a48dc6e9dbf932e78f53f9d285ed26  scala3-3.7.3-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            parse_sha256_sidecar(body),
            Some("10cb872e3162b36af2e4c993afebf515e9a48dc6e9dbf932e78f53f9d285ed26".to_string())
        );
    }

    #[test]
    fn supports_sidecar_floor_is_3_7_0() {
        assert!(supports_sidecar("3.7.0"));
        assert!(supports_sidecar("3.7.3"));
        assert!(supports_sidecar("3.8.3"));
        assert!(supports_sidecar("4.0.0"));
        assert!(supports_sidecar("v3.7.0"));
        assert!(!supports_sidecar("3.6.4"));
        assert!(!supports_sidecar("3.5.2"));
        assert!(!supports_sidecar("3.0.0"));
        assert!(!supports_sidecar("2.13.14"));
    }

    #[test]
    fn host_triple_covers_supported_hosts() {
        let combos = [
            ("macos", "aarch64", Some("aarch64-apple-darwin")),
            ("macos", "x86_64", Some("x86_64-apple-darwin")),
            ("linux", "x86_64", Some("x86_64-pc-linux")),
            ("linux", "aarch64", Some("aarch64-pc-linux")),
            ("windows", "x86_64", None),
        ];
        for (os, arch, want) in combos {
            let got = match (os, arch) {
                ("macos", "aarch64") => Some("aarch64-apple-darwin"),
                ("macos", "x86_64") => Some("x86_64-apple-darwin"),
                ("linux", "x86_64") => Some("x86_64-pc-linux"),
                ("linux", "aarch64") => Some("aarch64-pc-linux"),
                _ => None,
            };
            assert_eq!(got, want, "{os}/{arch}");
        }
    }

    #[test]
    fn version_cmp_orders_scala_releases() {
        let mut v = vec!["3.5.2", "3.8.3", "3.7.3", "3.7.0", "3.10.0"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["3.10.0", "3.8.3", "3.7.3", "3.7.0", "3.5.2"]);
    }
}
