//! Groovy backend — Apache zip distribution.
//!
//! Groovy is the second JVM-cross-backend backend (after Kotlin). Like
//! Kotlin, it declares `requires = ["java"]`; the orchestrator refuses
//! to install `[groovy]` unless `[java]` is also pinned, and `qusp run`
//! merges Java's PATH/JAVA_HOME into the env so `groovy`/`groovyc` find
//! a JRE.
//!
//! Source: Apache's stable archive. Same URL shape across all versions:
//!
//!   https://archive.apache.org/dist/groovy/<v>/distribution/apache-groovy-binary-<v>.zip
//!   https://archive.apache.org/dist/groovy/<v>/distribution/apache-groovy-binary-<v>.zip.sha256
//!
//! `archive.apache.org` keeps every version forever — we don't try to
//! play the dlcdn-vs-archive mirror dance. Slightly slower for the very
//! latest release, but qusp pin-stability beats download speed.
//!
//! Verification: hex-encoded sha256 in the sidecar (whitespace-trimmed
//! single line). Mandatory.
//!
//! Layout: zip extracts to `groovy-<v>/{bin/{groovy,groovysh,groovyc,...},
//! lib/, conf/}`. The store directory is symlinked to `data/groovy/<v>`
//! and `bin/` is on PATH.
//!
//! Tools: empty by design. Groovy's tooling model is Gradle/Maven; qusp
//! doesn't compete with build-tool plugin ecosystems.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct GroovyBackend;

const DIST_BASE: &str = "https://archive.apache.org/dist/groovy";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn groovy_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("groovy").join(version)
}

#[async_trait]
impl Backend for GroovyBackend {
    fn id(&self) -> &'static str {
        "groovy"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".groovy-version"]
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
            let f = d.join(".groovy-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".groovy-version".into(),
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
        let install_dir = groovy_root(&paths, version);
        if install_dir.join("bin").join("groovy").exists() {
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
        let asset = format!("apache-groovy-binary-{version}.zip");
        let asset_url = format!("{DIST_BASE}/{version}/distribution/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = parse_sha256_sidecar(&sha_text)
            .ok_or_else(|| anyhow!("empty .sha256 for {asset}"))?;

        let mut task = progress.start(&format!("downloading groovy {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded groovy {version}"));
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

        // Archive top-level is `groovy-<v>/`.
        let groovy_top = store_dir.join(format!("groovy-{version}"));
        if !groovy_top.join("bin").join("groovy").is_file() {
            bail!(
                "extracted Groovy archive did not contain groovy-{version}/bin/groovy at {}",
                groovy_top.display()
            );
        }

        // bin/* shipped without +x on macOS via some unzip impls.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for entry in std::fs::read_dir(groovy_top.join("bin"))? {
                let entry = entry?;
                let p = entry.path();
                if p.is_file() {
                    let mut perms = std::fs::metadata(&p)?.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    let _ = std::fs::set_permissions(&p, perms);
                }
            }
        }

        // bin/startGroovy on Darwin appends
        //   `-Xdock:icon=$GROOVY_HOME/lib/groovy.icns`
        // to `$JAVA_OPTS`, then later word-splits `$JAVA_OPTS` via an
        // *unquoted* expansion in `exec java`. When `$GROOVY_HOME`
        // contains a space — which is the macOS default for our data
        // root, `~/Library/Application Support/dev.O6lvl4.qusp` — the
        // tail of the icon path leaks out as a positional argument to
        // Java and is treated as the main-class, producing
        //   ClassNotFoundException: Support.dev.O6lvl4.qusp.groovy....
        // Strip the offending `-Xdock:icon=...` entry. We keep
        // `-Xdock:name=Groovy` (no spaces, no harm). The dock-icon
        // badge isn't worth a broken `groovy --version`.
        patch_startgroovy_darwin_dock(&groovy_top.join("bin").join("startGroovy"))?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&groovy_top, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), groovy_top.display())
            })?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = groovy_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("groovy {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("groovy");
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
        // archive.apache.org publishes a directory listing per
        // `<base>/groovy/`. Each release is a `<X.Y.Z>/` row.
        let url = format!("{DIST_BASE}/");
        let body = http
            .get_text(&url)
            .await
            .context("fetch archive.apache.org/dist/groovy/")?;
        let mut versions = parse_apache_dir_versions(&body);
        versions.sort_by(|a, b| version_cmp(b, a));
        versions.dedup();
        Ok(versions)
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = groovy_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("GROOVY_HOME".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("groovy"),
            FarmBinary::unversioned("groovyc"),
            FarmBinary::unversioned("groovysh"),
            FarmBinary::unversioned("groovyConsole"),
            FarmBinary::unversioned("grape"),
        ]
    }
}

fn parse_sha256_sidecar(s: &str) -> Option<String> {
    s.split_whitespace().next().map(|x| x.to_string())
}

/// Strip the `-Xdock:icon=$GROOVY_HOME/lib/groovy.icns` flag from
/// startGroovy's Darwin block. See call site for the full explanation.
/// Idempotent — if the substring isn't present, leaves the file alone.
fn patch_startgroovy_darwin_dock(path: &Path) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .with_context(|| format!("read {} for dock-icon patch", path.display()))?;
    let needle = " -Xdock:icon=$GROOVY_HOME/lib/groovy.icns";
    if !original.contains(needle) {
        return Ok(());
    }
    let patched = original.replace(needle, "");
    std::fs::write(path, patched)
        .with_context(|| format!("write {} after dock-icon patch", path.display()))?;
    Ok(())
}

/// Apache mod_autoindex listings render each child as
/// `<a href="X.Y.Z/">X.Y.Z/</a>`. Pull every `href="<vers>/"`.
fn parse_apache_dir_versions(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in html.lines() {
        // Cheap pattern: find `href="`, take until `/"`.
        let mut rest = line;
        while let Some(idx) = rest.find("href=\"") {
            rest = &rest[idx + 6..];
            if let Some(end) = rest.find("\"") {
                let candidate = &rest[..end];
                rest = &rest[end + 1..];
                if let Some(stripped) = candidate.strip_suffix('/') {
                    if looks_like_version(stripped) {
                        out.push(stripped.to_string());
                    }
                }
            } else {
                break;
            }
        }
    }
    out
}

fn looks_like_version(s: &str) -> bool {
    // X.Y.Z (allow alpha/beta/rc suffix on Z).
    let mut parts = s.split('.');
    let a = parts.next();
    let b = parts.next();
    let c = parts.next();
    let extra = parts.next();
    if extra.is_some() {
        return false;
    }
    match (a, b, c) {
        (Some(a), Some(b), Some(c)) => {
            a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
                && c.chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c.is_ascii_alphabetic())
                && !c.is_empty()
        }
        _ => false,
    }
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
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
    fn parses_apache_dir_listing() {
        let html = r#"
            <a href="4.0.21/">4.0.21/</a>
            <a href="4.0.22/">4.0.22/</a>
            <a href="5.0.0-alpha-1/">5.0.0-alpha-1/</a>
            <a href="../">parent</a>
            <a href="not-a-version/">junk</a>
        "#;
        let mut got = parse_apache_dir_versions(html);
        got.sort();
        assert_eq!(got, vec!["4.0.21", "4.0.22", "5.0.0-alpha-1"]);
    }

    #[test]
    fn version_cmp_orders_apache_releases() {
        let mut v = vec!["4.0.22", "3.0.21", "4.0.21", "5.0.0", "2.5.23"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["5.0.0", "4.0.22", "4.0.21", "3.0.21", "2.5.23"]);
    }

    #[test]
    fn looks_like_version_filters_tail_paths() {
        assert!(looks_like_version("4.0.22"));
        assert!(looks_like_version("5.0.0-alpha-1"));
        assert!(!looks_like_version("docs"));
        assert!(!looks_like_version(".."));
        assert!(!looks_like_version("4.0.22.1"));
    }

    #[test]
    fn dock_icon_patch_strips_only_the_offending_arg() {
        let tmp = std::env::temp_dir().join("qusp-groovy-startgroovy-test.sh");
        let original = "    JAVA_OPTS=\"$JAVA_OPTS -Xdock:name=$GROOVY_APP_NAME -Xdock:icon=$GROOVY_HOME/lib/groovy.icns\"\n";
        std::fs::write(&tmp, original).unwrap();
        patch_startgroovy_darwin_dock(&tmp).unwrap();
        let after = std::fs::read_to_string(&tmp).unwrap();
        assert_eq!(
            after,
            "    JAVA_OPTS=\"$JAVA_OPTS -Xdock:name=$GROOVY_APP_NAME\"\n"
        );
        // Idempotent: second patch is a no-op.
        patch_startgroovy_darwin_dock(&tmp).unwrap();
        assert_eq!(after, std::fs::read_to_string(&tmp).unwrap());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn parses_sha256_sidecar_trim() {
        assert_eq!(
            parse_sha256_sidecar("d91a3ddfe353871d4c2656d3d0a05c828bc3ff36e9d49dbdbec13dcd98f05877\n"),
            Some("d91a3ddfe353871d4c2656d3d0a05c828bc3ff36e9d49dbdbec13dcd98f05877".to_string())
        );
        assert_eq!(parse_sha256_sidecar(""), None);
    }
}
