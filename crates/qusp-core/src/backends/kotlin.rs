//! Kotlin backend — JVM-target compiler.
//!
//! qusp's first **cross-backend dependency**. Kotlin/JVM requires a
//! JDK to run kotlinc and compile/execute output. The backend declares
//! that explicitly via `Backend::requires`:
//!
//! ```rust,ignore
//! fn requires(&self) -> &[&'static str] { &["java"] }
//! ```
//!
//! The orchestrator validates the requirement before any install runs:
//! `[kotlin]` pinned without `[java]` errors with a clear message.
//! `build_run_env` outputs PATH for kotlinc; the orchestrator merges
//! it with the Java backend's PATH/JAVA_HOME so kotlinc and friends
//! resolve `java` correctly out of the qusp-managed JDK.
//!
//! Source: GitHub releases of `JetBrains/kotlin`, `kotlin-compiler-X.Y.Z.zip`
//! verified against the per-asset `.sha256` sidecar. Single archive
//! covers all hosts (the compiler is platform-independent JAR-bundled).
//!
//! Kotlin/Native is intentionally out of scope for v0.9.0 — separate
//! tarballs per host triple, much larger surface area, the JVM target
//! is the dominant use case.
//!
//! Tools: empty by design. Kotlin's tooling model is Gradle-driven;
//! qusp doesn't compete with Gradle plugins.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct KotlinBackend;

const REPO: &str = "JetBrains/kotlin";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn kotlin_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("kotlin").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

#[async_trait]
impl Backend for KotlinBackend {
    fn id(&self) -> &'static str {
        "kotlin"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".kotlin-version", "build.gradle.kts", "build.gradle"]
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
            let f = d.join(".kotlin-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".kotlin-version".into(),
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
        let install_dir = kotlin_root(&paths, version);
        if install_dir.join("bin").join("kotlinc").exists() {
            return Ok(InstallReport {
                version: strip_v(version).to_string(),
                install_dir,
                already_present: true,
            });
        }

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;
        let v = strip_v(version);
        let tag = format!("v{v}");
        let asset = format!("kotlin-compiler-{v}.zip");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = sha_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty .sha256 for {asset}"))?
            .to_string();

        let mut task = progress.start(&format!("downloading kotlin {v}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded kotlin {v}"));
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

        // Archive expands to `kotlinc/{bin,lib,license}`.
        let kotlinc = store_dir.join("kotlinc");
        if !kotlinc.join("bin").join("kotlinc").is_file() {
            bail!(
                "extracted Kotlin archive did not contain kotlinc/bin/kotlinc at {}",
                kotlinc.display()
            );
        }

        // Kotlin's bin scripts ship without the executable bit on macOS
        // when extracted by some unzip implementations. Restore +x.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(kotlinc.join("bin"))? {
                let entry = entry?;
                let p = entry.path();
                if p.is_file() {
                    let mut perms = std::fs::metadata(&p)?.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    let _ = std::fs::set_permissions(&p, perms);
                }
            }
        }

        // Upstream kotlinc has an unquoted $(dirname ...) that breaks
        // when the install path contains spaces (e.g. "Application Support").
        // Patch it so the farm symlink works from any location.
        patch_kotlinc_spaces(&kotlinc)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&kotlinc, &install_dir).with_context(|| {
            format!("symlink {} → {}", install_dir.display(), kotlinc.display())
        })?;

        Ok(InstallReport {
            version: v.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = kotlin_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("kotlin {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("kotlin");
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
            serde_json::from_str(&body).context("parse JetBrains/kotlin release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| strip_v(&r.tag_name).to_string())
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
            "Kotlin's tool ecosystem is Gradle-driven (KSP, ksp-gradle-plugin, \
             dokka, …). qusp doesn't curate a tool registry — declare these as \
             Gradle dependencies in build.gradle.kts. '{name}' has no qusp \
             install path on the kotlin backend."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = kotlin_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("KOTLIN_HOME".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("kotlin"),
            FarmBinary::unversioned("kotlinc"),
            FarmBinary::unversioned("kotlinc-jvm"),
            FarmBinary::unversioned("kotlinc-js"),
            FarmBinary::unversioned("kotlin-jvm"),
            FarmBinary::unversioned("kotlin-js"),
            FarmBinary::unversioned("kapt"),
        ]
    }
}

/// Patch upstream kotlinc's `findKotlinHome` to quote `$(dirname ...)`,
/// fixing `cd: too many arguments` on paths with spaces.
fn patch_kotlinc_spaces(kotlinc_dir: &Path) -> Result<()> {
    let script = kotlinc_dir.join("bin").join("kotlinc");
    if !script.is_file() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&script)?;
    let bad = r#"cd -P $(dirname "$source") && cd -P $(dirname "$linked")"#;
    if content.contains(bad) {
        let fixed = content.replace(
            bad,
            r#"cd -P "$(dirname "$source")" && cd -P "$(dirname "$linked")"#,
        );
        std::fs::write(&script, fixed)?;
    }
    Ok(())
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}
