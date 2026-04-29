//! Bun backend — native installer, toolchain only.
//!
//! `bun` ships as a single self-contained binary from oven-sh/bun GitHub
//! releases. qusp downloads `bun-<triple>.zip`, verifies it against the
//! release's `SHASUMS256.txt`, extracts the binary, and symlinks
//! `versions/bun/<v>` at it.
//!
//! No tools by design. Bun has its own npm-compatible `bun install`
//! for packages — qusp doesn't shadow that. For qusp-managed Node-side
//! CLIs (pnpm, prettier, …) use the `node` backend.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct BunBackend;

const REPO: &str = "oven-sh/bun";

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-aarch64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn bun_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("bun").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    let v = v.strip_prefix("bun-").unwrap_or(v);
    v.strip_prefix('v').unwrap_or(v)
}

#[derive(Deserialize, Debug)]
struct GhRelease {
    tag_name: String,
}

#[async_trait]
impl Backend for BunBackend {
    fn id(&self) -> &'static str {
        "bun"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["bun.lockb", "bun.lock", ".bun-version", "package.json"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".bun-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".bun-version".into(),
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
        let install_dir = bun_root(&paths, version);
        if install_dir.join("bin").join("bun").exists() {
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
        let triple =
            target_triple().ok_or_else(|| anyhow!("oven-sh/bun has no asset for this platform"))?;
        let v_strip = strip_v(version);
        // Bun tags are `bun-vX.Y.Z`.
        let tag = format!("bun-v{v_strip}");
        let asset = format!("bun-{triple}.zip");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sums_url = format!("https://github.com/{REPO}/releases/download/{tag}/SHASUMS256.txt");

        let sums_text = http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = crate::backends::node::parse_shasums_line(&sums_text, &asset)
            .ok_or_else(|| anyhow!("no entry for {asset} in SHASUMS256.txt"))?;

        let mut task = progress.start(&format!("downloading bun {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded bun {version}"));
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if expected != actual {
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

        // The zip extracts a single `bun-<triple>/` dir containing `bun`.
        let inner = store_dir.join(format!("bun-{triple}"));
        let bun_bin = if inner.join("bun").is_file() {
            inner.join("bun")
        } else if store_dir.join("bun").is_file() {
            store_dir.join("bun")
        } else {
            bail!(
                "extracted Bun archive did not contain a `bun` binary at {} or {}/bun",
                inner.display(),
                store_dir.display()
            );
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&bun_bin)?.permissions();
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(&bun_bin, perms)?;
        }
        // Lay out a `bin/` so build_run_env's path_prepend targets a
        // directory rather than a single file.
        let bin_dir = store_dir.join("bin");
        anyv_core::paths::ensure_dir(&bin_dir)?;
        let bin_link = bin_dir.join("bun");
        if bin_link.exists() || bin_link.is_symlink() {
            let _ = std::fs::remove_file(&bin_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&bun_bin, &bin_link)?;
        #[cfg(windows)]
        std::fs::copy(&bun_bin, &bin_link)?;
        // bunx is symlinked to bun upstream; mirror that.
        let bunx_link = bin_dir.join("bunx");
        if bunx_link.exists() || bunx_link.is_symlink() {
            let _ = std::fs::remove_file(&bunx_link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&bun_bin, &bunx_link)?;
        #[cfg(windows)]
        std::fs::copy(&bun_bin, &bunx_link)?;

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
        let dir = bun_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("bun {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("bun");
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
            serde_json::from_str(&body).context("parse oven-sh/bun release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .map(|r| strip_v(&r.tag_name).to_string())
            .filter(|v| !v.starts_with("canary"))
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
            "Bun doesn't have a qusp-managed tool registry. Use Bun's own `bun install` \
             for npm packages, or pin Node-side CLIs under `[node.tools]` to share them \
             across runtimes. '{name}' has no qusp install path on the bun backend."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = bun_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("bun"),
            FarmBinary::unversioned("bunx"),
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
