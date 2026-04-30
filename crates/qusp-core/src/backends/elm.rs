//! Elm backend — single-binary compiler.
//!
//! Elm ships as a single gzipped binary from `elm/compiler` GitHub
//! releases. The asset is a bare `.gz` (not `.tar.gz`), so we
//! decompress with `flate2` directly rather than `extract_archive`.
//!
//! **Checksum note**: upstream does not publish sha256 sidecar files.
//! qusp computes sha256 of the downloaded archive for content-addressing
//! but cannot verify against a publisher-published hash. HTTPS +
//! GitHub release integrity is the trust anchor.
//!
//! Toolchain only — no curated tool registry. `elm-format` and
//! `elm-test` are typically installed via npm (`npx elm-format`) or
//! direct download; qusp doesn't shadow those.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct ElmBackend;

const REPO: &str = "elm/compiler";

fn asset_name() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "binary-for-mac-64-bit-ARM.gz",
        ("macos", "x86_64") => "binary-for-mac-64-bit.gz",
        ("linux", "x86_64") => "binary-for-linux-64-bit.gz",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn elm_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("elm").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

#[async_trait]
impl Backend for ElmBackend {
    fn id(&self) -> &'static str {
        "elm"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["elm.json"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        // Elm projects have elm.json with an "elm-version" field.
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join("elm.json");
            if f.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&f) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if let Some(ver) = v.get("elm-version").and_then(|v| v.as_str()) {
                            return Ok(Some(DetectedVersion {
                                version: strip_v(ver).to_string(),
                                source: "elm.json".into(),
                                origin: f,
                            }));
                        }
                    }
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
        let install_dir = elm_root(&paths, version);
        if install_dir.join("bin").join("elm").exists() {
            return Ok(InstallReport {
                version: strip_v(version).to_string(),
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;
        let asset =
            asset_name().ok_or_else(|| anyhow!("elm/compiler has no binary for this platform"))?;
        let v_strip = strip_v(version);
        let asset_url = format!("https://github.com/{REPO}/releases/download/{v_strip}/{asset}");

        let mut task = progress.start(&format!("downloading elm {v_strip}"), None);
        let gz_bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded elm {v_strip}"));

        // Decompress gzip → raw binary.
        let mut decoder = flate2::read::GzDecoder::new(&gz_bytes[..]);
        let mut bin_bytes = Vec::new();
        decoder
            .read_to_end(&mut bin_bytes)
            .context("decompress elm binary from .gz")?;

        // Content-address the binary (no upstream hash to verify against).
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bin_bytes);
        let hash = hex::encode(hasher.finalize());

        let store_dir = paths.store().join(&hash[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        let elm_bin = store_dir.join("elm");
        std::fs::write(&elm_bin, &bin_bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&elm_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&elm_bin, perms)?;
        }

        // Create bin/ subdir with symlink for consistent layout.
        let bin_dir = store_dir.join("bin");
        anyv_core::paths::ensure_dir(&bin_dir)?;
        let bin_link = bin_dir.join("elm");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&elm_bin, &bin_link)?;
        #[cfg(windows)]
        std::fs::copy(&elm_bin, &bin_link)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&store_dir, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                store_dir.display()
            )
        })?;

        Ok(InstallReport {
            version: v_strip.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = elm_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("elm {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("elm");
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
            #[serde(default)]
            prerelease: bool,
        }
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse elm/compiler release index")?;
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
            "Elm's tool ecosystem is npm-driven (elm-format, elm-test, elm-review). \
             Install them via `npm install -g {name}` or `npx {name}`. \
             qusp doesn't curate an Elm tool registry."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = elm_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: std::collections::BTreeMap::new(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![FarmBinary::unversioned("elm")]
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
