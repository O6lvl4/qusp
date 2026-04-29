//! Deno backend — native installer.
//!
//! Toolchain only. `deno` ships as a single self-contained binary, so
//! qusp downloads `deno-<triple>.zip` from `denoland/deno` GitHub
//! releases, verifies it against the per-asset `.sha256sum` sidecar,
//! extracts the binary into a content-addressed store, and symlinks
//! `versions/deno/<v>` at it.
//!
//! No tools by design. Deno's tooling model is per-script URLs (or
//! `deno install -g` writing to `~/.deno/bin`) — qusp doesn't try to
//! shadow either. `qusp add tool …` for deno gives a clear error
//! pointing the user back at `deno install` / per-project pinning in
//! `deno.json`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct DenoBackend;

const REPO: &str = "denoland/deno";

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn deno_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("deno").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

#[derive(Deserialize, Debug)]
struct GhRelease {
    tag_name: String,
}

#[async_trait]
impl Backend for DenoBackend {
    fn id(&self) -> &'static str {
        "deno"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["deno.json", "deno.jsonc", ".deno-version"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".deno-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".deno-version".into(),
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
        let install_dir = deno_root(&paths, version);
        if install_dir.join("bin").join("deno").exists() {
            return Ok(InstallReport {
                version: strip_v(version).to_string(),
                install_dir,
                already_present: true,
            });
        }

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard = crate::effects::StoreLock::acquire(
            &crate::effects::lock_path_for(&install_dir),
        )?;
        let triple = target_triple()
            .ok_or_else(|| anyhow!("denoland/deno has no asset for this platform"))?;
        let v_strip = strip_v(version);
        // Deno tags are `vX.Y.Z`.
        let tag = format!("v{v_strip}");
        let asset = format!("deno-{triple}.zip");
        let sums_asset = format!("deno-{triple}.sha256sum");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sums_url = format!("https://github.com/{REPO}/releases/download/{tag}/{sums_asset}");

        let sums_text = http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = sums_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty .sha256sum response for {asset}"))?
            .to_string();

        let mut task = progress.start(&format!("downloading deno {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded deno {version}"));

        let cache_path = paths.cache.join(&asset);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        // Stage extraction in cache so we can sha256-verify the inner
        // binary before promoting to the content-addressed store.
        let stage = paths.cache.join(format!("deno-stage-{v_strip}"));
        if stage.exists() {
            std::fs::remove_dir_all(&stage).ok();
        }
        anyv_core::paths::ensure_dir(&stage)?;
        extract_archive(&cache_path, &stage)?;

        let staged_bin = stage.join("deno");
        if !staged_bin.is_file() {
            bail!(
                "extracted archive did not contain a top-level `deno` binary at {}",
                staged_bin.display()
            );
        }
        // The .sha256sum sidecar carries the hash of the *inner* binary
        // (line is `<hash>  deno`), not the .zip wrapper.
        let inner_bytes = std::fs::read(&staged_bin)?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&inner_bytes);
        let actual = hex::encode(hasher.finalize());
        if expected != actual {
            let _ = std::fs::remove_dir_all(&stage);
            bail!("sha256 mismatch for inner deno binary: expected {expected}, got {actual}");
        }

        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        let deno_bin = store_dir.join("deno");
        std::fs::rename(&staged_bin, &deno_bin)
            .or_else(|_| std::fs::copy(&staged_bin, &deno_bin).map(|_| ()))?;
        let _ = std::fs::remove_dir_all(&stage);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&deno_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&deno_bin, perms)?;
        }
        // Build a `bin/` subdir inside the store entry so `path_prepend`
        // can target a directory rather than the binary itself.
        let bin_dir = store_dir.join("bin");
        anyv_core::paths::ensure_dir(&bin_dir)?;
        let bin_link = bin_dir.join("deno");
        if bin_link.exists() || bin_link.is_symlink() {
            let _ = std::fs::remove_file(&bin_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&deno_bin, &bin_link)?;
        #[cfg(windows)]
        std::fs::copy(&deno_bin, &bin_link)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&store_dir, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), store_dir.display())
            })?;

        let _ = std::fs::remove_file(&cache_path);
        Ok(InstallReport {
            version: v_strip.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = deno_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("deno {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("deno");
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
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<GhRelease> =
            serde_json::from_str(&body).context("parse denoland/deno release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
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
            "Deno doesn't have a global tool registry compatible with `qusp add tool`. \
             Two paths: (1) per-project — pin the tool's URL in deno.json's `imports` \
             map and run via `deno run <alias>`. (2) global — `deno install -g <url>` \
             writes to ~/.deno/bin (qusp doesn't shadow it). \
             '{name}' was passed but has no qusp-managed install path."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = deno_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("deno"),
        ]
    }
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
