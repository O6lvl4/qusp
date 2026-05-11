//! Clojure backend — direct release tarball, official Clojure CLI.
//!
//! The roadmap originally proposed a Coursier wrap (shared with Scala).
//! Scala v0.20.0 dropped Coursier in favour of direct GitHub releases,
//! and Clojure can do the same — `clojure/brew-install` publishes a
//! single host-independent `clojure-tools-<v>.tar.gz` plus a `.sha256`
//! sidecar.
//!
//! Source:
//!   https://github.com/clojure/brew-install/releases/download/<v>/clojure-tools-<v>.tar.gz
//!   https://github.com/clojure/brew-install/releases/download/<v>/clojure-tools-<v>.tar.gz.sha256
//!
//! Verification: the sidecar is bare hex (one whitespace-separated
//! token, no filename suffix in this case). Mandatory.
//!
//! Layout: tarball expands to `clojure-tools/{clojure, clj, deps.edn,
//! example-deps.edn, tools.edn, exec.jar, clojure-tools-<v>.jar,
//! clojure.1, clj.1, install.sh}` — all in one flat directory. The
//! upstream `posix-install.sh` reorganises them into a FHS layout
//! (`<prefix>/bin/{clj,clojure}`, `<prefix>/lib/clojure/...`) and
//! sed-substitutes `PREFIX` / `BINDIR` placeholders inside the launcher
//! scripts. We replicate that logic in pure Rust so qusp stays in
//! charge of the install — no `posix-install.sh` subprocess.
//!
//! Cross-backend dep: `requires = ["java"]`. The `clj` / `clojure`
//! launchers `exec` `java -cp <libexec/exec.jar:libexec/clojure-tools-<v>.jar:...>`
//! against the JDK on PATH; without `[java]` pinned the launcher fails
//! at run time with a generic "java: command not found", so the
//! orchestrator catches the missing dep before any install runs.
//!
//! Tools: empty by design. Clojure's package model is `deps.edn` with
//! Maven coords; qusp doesn't curate against it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;

use super::common;
use crate::backend::*;

pub struct ClojureBackend;

const REPO: &str = "clojure/brew-install";

#[async_trait]
impl Backend for ClojureBackend {
    fn id(&self) -> &'static str {
        "clojure"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".clojure-version", "deps.edn"]
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
            let f = d.join(".clojure-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".clojure-version".into(),
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

        let paths = common::qusp_paths()?;
        paths.ensure_dirs()?;
        let install_dir = common::lang_root(&paths, "clojure", version);
        if let Some(report) = common::check_already_installed(&install_dir, "bin/clj", version) {
            return Ok(report);
        }

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard = common::acquire_install_lock(&install_dir)?;
        let asset = format!("clojure-tools-{version}.tar.gz");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{version}/{asset}");
        let sha_url = format!("{asset_url}.sha256");

        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = parse_sha256_sidecar(&sha_text)
            .ok_or_else(|| anyhow!("could not parse sha256 from sidecar for {asset}"))?;

        let mut task = progress.start(&format!("downloading clojure {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded clojure {version}"));
        common::verify_sha256(&bytes, &expected, &asset)?;

        let store_dir = common::stage_to_store(&paths, &bytes, &expected, &asset)?;

        // Tarball expands to `clojure-tools/...` (flat).
        let stage = store_dir.join("clojure-tools");
        if !stage.is_dir() {
            bail!(
                "extracted Clojure archive did not contain clojure-tools/ at {}",
                stage.display()
            );
        }

        // posix-install.sh reorganises the flat tarball into:
        //   <prefix>/bin/clj            (BINDIR substituted in launcher)
        //   <prefix>/bin/clojure        (PREFIX substituted in launcher)
        //   <prefix>/lib/clojure/{deps.edn, example-deps.edn, tools.edn}
        //   <prefix>/lib/clojure/libexec/{exec.jar, clojure-tools-<v>.jar}
        //   <prefix>/share/man/man1/{clj.1, clojure.1}
        //
        // We rebuild that layout in `prefix/` *inside* the store dir,
        // then symlink `data/clojure/<v>/` to `prefix/`.
        let prefix = store_dir.join("prefix");
        let bin_dir = prefix.join("bin");
        let lib_dir = prefix.join("lib").join("clojure");
        let libexec_dir = lib_dir.join("libexec");
        let man_dir = prefix.join("share").join("man").join("man1");
        for d in [&bin_dir, &libexec_dir, &man_dir] {
            anyv_core::paths::ensure_dir(d)?;
        }
        // edn + jars (mode 644 in upstream — for qusp's purposes the
        // default copy mode is fine on POSIX; symlink targets can't
        // change perms anyway).
        for (src, dst) in [
            ("deps.edn", lib_dir.join("deps.edn")),
            ("example-deps.edn", lib_dir.join("example-deps.edn")),
            ("tools.edn", lib_dir.join("tools.edn")),
            ("exec.jar", libexec_dir.join("exec.jar")),
        ] {
            std::fs::copy(stage.join(src), &dst)
                .with_context(|| format!("copy {} → {}", src, dst.display()))?;
        }
        // The versioned tools jar.
        let tools_jar = format!("clojure-tools-{version}.jar");
        std::fs::copy(stage.join(&tools_jar), libexec_dir.join(&tools_jar))
            .with_context(|| format!("copy {tools_jar}"))?;

        // Sed-substitute the launcher placeholders. Critical detail:
        // the substitution targets a bare assignment line
        //   install_dir=PREFIX
        // and `prefix/lib/clojure` lives under macOS's
        // `~/Library/Application Support/dev.O6lvl4.qusp/...`, which
        // contains a literal space. An unquoted substitution
        //   install_dir=/Users/.../Application Support/...
        // word-splits in bash and the launcher fails with
        //   "Support/.../clojure: No such file or directory"
        // (the same Application-Support trap that bit Groovy in
        // v0.18.0). Wrap the path in single-quotes during substitution
        // so the resulting shell line is `install_dir='<path>'`,
        // which is safe for any character except `'` itself —
        // qusp data paths don't contain single quotes by construction.
        let lib_dir_quoted = crate::effects::shell_single_quote(&lib_dir.to_string_lossy());
        let bin_dir_quoted = crate::effects::shell_single_quote(&bin_dir.to_string_lossy());
        let clojure_src =
            std::fs::read_to_string(stage.join("clojure")).context("read clojure-tools/clojure")?;
        let clj_src =
            std::fs::read_to_string(stage.join("clj")).context("read clojure-tools/clj")?;
        std::fs::write(
            bin_dir.join("clojure"),
            clojure_src.replace("PREFIX", &lib_dir_quoted),
        )?;
        std::fs::write(
            bin_dir.join("clj"),
            clj_src.replace("BINDIR", &bin_dir_quoted),
        )?;

        // Man pages — nice-to-have, replicating upstream behaviour.
        for (src, dst) in [
            ("clojure.1", man_dir.join("clojure.1")),
            ("clj.1", man_dir.join("clj.1")),
        ] {
            std::fs::copy(stage.join(src), &dst).ok();
        }

        // chmod +x the launchers (post-substitution).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["clojure", "clj"] {
                let p = bin_dir.join(name);
                let mut perms = std::fs::metadata(&p)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&p, perms)?;
            }
        }

        common::finalize_install(&prefix, &install_dir)?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        common::uninstall_version("clojure", version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        common::list_installed_versions("clojure")
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
            serde_json::from_str(&body).context("parse clojure/brew-install release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| r.tag_name.trim_start_matches('v').to_string())
            .collect();
        out.sort_by(|a, b| common::version_cmp(b, a));
        Ok(out)
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = common::qusp_paths()?;
        let root = common::lang_root(&paths, "clojure", version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("CLOJURE_HOME".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("clojure"),
            FarmBinary::unversioned("clj"),
        ]
    }
}

fn parse_sha256_sidecar(s: &str) -> Option<String> {
    s.split_whitespace().next().map(|x| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clojure_sha256_sidecar() {
        // Real sidecar from clojure/brew-install: bare hex on one line
        // (no `*<filename>` suffix, unlike Dart's BSD-style file).
        let body = "13769da6d63a98deb2024378ae1a64e4ee211ac1035340dfca7a6944c41cde21\n";
        assert_eq!(
            parse_sha256_sidecar(body),
            Some("13769da6d63a98deb2024378ae1a64e4ee211ac1035340dfca7a6944c41cde21".to_string())
        );
    }

    // shell_single_quote tests moved to crates/qusp-core/src/effects/space_trap.rs
    // when the helper was consolidated in v0.28.1.

    #[test]
    fn version_cmp_orders_clojure_4_segment() {
        let mut v = vec!["1.12.4.1618", "1.12.0.1530", "1.11.4.1474", "1.12.4.1500"];
        v.sort_by(|a, b| common::version_cmp(b, a));
        assert_eq!(
            v,
            vec!["1.12.4.1618", "1.12.4.1500", "1.12.0.1530", "1.11.4.1474",]
        );
    }
}
