//! Almide backend — functional language optimized for LLM code generation.
//!
//! Single binary from `almide/almide` GitHub releases. Verified against
//! the combined `almide-checksums.sha256` published alongside each
//! release. Assets are named `almide-${os}-${arch}.tar.gz` and unpack
//! to `almide-${os}-${arch}/almide`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct AlmideBackend;

const REPO: &str = "almide/almide";

/// Map a Rust `(os, arch)` pair to Almide's release naming.
///
/// Upstream uses `${os}-${arch}` with `os ∈ {macos, linux}` and
/// `arch ∈ {x86_64, aarch64}`. No Windows builds at v0.15.x.
fn platform() -> Option<(&'static str, &'static str)> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macos", "aarch64"),
        ("macos", "x86_64") => ("macos", "x86_64"),
        ("linux", "x86_64") => ("linux", "x86_64"),
        ("linux", "aarch64") => ("linux", "aarch64"),
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn almide_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("almide").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

#[async_trait]
impl Backend for AlmideBackend {
    fn id(&self) -> &'static str {
        "almide"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["almide.toml"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".almide-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".almide-version".into(),
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

        // `latest` resolves against the GitHub release index. We do this
        // first so we hit the cached store path on subsequent calls.
        let resolved = if version == "latest" {
            self.list_remote(http)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("no almide releases found upstream"))?
        } else {
            strip_v(version).to_string()
        };
        let version = resolved.as_str();

        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = almide_root(&paths, version);
        if install_dir.join("bin").join("almide").exists() {
            return Ok(InstallReport {
                version: strip_v(version).to_string(),
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;
        let (os, arch) =
            platform().ok_or_else(|| anyhow!("almide has no binary for this platform"))?;
        let v_strip = strip_v(version);
        let tag = format!("v{v_strip}");
        let stem = format!("almide-{os}-{arch}");
        let asset = format!("{stem}.tar.gz");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sha_url =
            format!("https://github.com/{REPO}/releases/download/{tag}/almide-checksums.sha256");

        // Combined checksums file: parse the `<sha>  <filename>` line for
        // the asset we want.
        let sha_text = http
            .get_text(&sha_url)
            .await
            .with_context(|| format!("fetch {sha_url}"))?;
        let expected = sha_text
            .lines()
            .find_map(|line| {
                let mut it = line.split_whitespace();
                let hash = it.next()?;
                let name = it.next()?;
                if name.trim_start_matches('*') == asset {
                    Some(hash.to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("no checksum for {asset} in almide-checksums.sha256"))?;

        let mut task = progress.start(&format!("downloading almide {v_strip}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded almide {v_strip}"));

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

        // Archive lays out `almide-${os}-${arch}/almide`.
        let almide_bin = store_dir.join(&stem).join("almide");
        if !almide_bin.is_file() {
            bail!(
                "extracted archive did not contain `almide` at {}",
                almide_bin.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&almide_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&almide_bin, perms)?;
        }

        // Surface a stable bin/ subdirectory for the rest of qusp.
        let bin_dir = store_dir.join("bin");
        anyv_core::paths::ensure_dir(&bin_dir)?;
        let bin_link = bin_dir.join("almide");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&almide_bin, &bin_link)?;
        #[cfg(windows)]
        std::fs::copy(&almide_bin, &bin_link)?;

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
        let dir = almide_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("almide {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("almide");
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
            serde_json::from_str(&body).context("parse almide/almide release index")?;
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
            "Almide manages dependencies via `[dependencies]` in almide.toml \
             (typically git refs to almide packages). qusp does not curate \
             a separate Almide tool registry. Add `{name}` to your almide.toml."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = almide_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: std::collections::BTreeMap::new(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![FarmBinary::unversioned("almide")]
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
