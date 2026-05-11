//! Zig backend.
//!
//! Toolchain comes from `ziglang.org/download/index.json` — a single
//! JSON document keyed by version, each entry keyed by host triple
//! with `{ tarball, shasum, size }`. The `shasum` is sha256 of the
//! `.tar.xz` bytes, inline in the index. No separate sidecar fetch.
//!
//! Tools: empty by design. Zig's package management is per-project
//! `build.zig.zon` (handled by `zig build` itself) — qusp doesn't
//! shadow that.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use super::common;
use crate::backend::*;

pub struct ZigBackend;

const INDEX_URL: &str = "https://ziglang.org/download/index.json";

fn target_triple() -> Option<&'static str> {
    Some(match common::os_arch() {
        ("macos", "aarch64") => "aarch64-macos",
        ("macos", "x86_64") => "x86_64-macos",
        ("linux", "x86_64") => "x86_64-linux",
        ("linux", "aarch64") => "aarch64-linux",
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct ZigAsset {
    pub(crate) tarball: String,
    pub(crate) shasum: String,
}

#[async_trait]
impl Backend for ZigBackend {
    fn id(&self) -> &'static str {
        "zig"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".zig-version", "build.zig", "build.zig.zon"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".zig-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".zig-version".into(),
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
        ctx: &InstallCtx<'_>,
    ) -> Result<InstallReport> {
        let paths = common::qusp_paths()?;
        paths.ensure_dirs()?;
        let install_dir = common::lang_root(&paths, "zig", version);

        if let Some(report) = common::check_already_installed(&install_dir, "zig", version) {
            return Ok(report);
        }
        let _guard = common::acquire_install_lock(&install_dir)?;

        let triple =
            target_triple().ok_or_else(|| anyhow!("ziglang.org has no asset for this platform"))?;
        let body = ctx
            .http
            .get_text(INDEX_URL)
            .await
            .with_context(|| format!("fetch {INDEX_URL}"))?;
        let asset = pick_zig_asset(&body, version, triple)?
            .ok_or_else(|| anyhow!("no Zig asset for {version} on {triple}"))?;

        let bytes = common::download_and_verify(
            ctx.http,
            &asset.tarball,
            &asset.shasum,
            ctx.progress,
            &format!("zig {version}"),
        )
        .await?;

        // Zig uses .tar.xz — custom extractor needed (not handled by anyv extract_archive).
        let cache_name = format!("zig-{version}-{triple}.tar.xz");
        let cache_path = paths.cache.join(&cache_name);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;

        let sha_hex = hex::encode(sha2::Digest::finalize(sha2::Sha256::new_with_prefix(
            &bytes,
        )));
        let store_dir = paths.store().join(&sha_hex[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_tar_xz(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        let inner = find_zig_top(&store_dir)?;
        common::finalize_install(&inner, &install_dir)?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        common::uninstall_version("zig", version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        common::list_installed_versions("zig")
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let body = http.get_text(INDEX_URL).await?;
        Ok(list_zig_versions(&body))
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = common::qusp_paths()?;
        let root = common::lang_root(&paths, "zig", version);
        Ok(RunEnv {
            path_prepend: vec![root],
            env: Default::default(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![FarmBinary::unversioned_flat("zig")]
    }
}

/// Pure: pick `(tarball_url, sha256)` for the given version + triple.
pub(crate) fn pick_zig_asset(
    index_body: &str,
    version: &str,
    triple: &str,
) -> Result<Option<ZigAsset>> {
    let root: serde_json::Value =
        serde_json::from_str(index_body).context("parse Zig index.json")?;
    let Some(version_entry) = root.get(version) else {
        return Ok(None);
    };
    let Some(asset_entry) = version_entry.get(triple) else {
        return Ok(None);
    };
    let asset: ZigAsset =
        serde_json::from_value(asset_entry.clone()).context("parse Zig asset entry")?;
    Ok(Some(asset))
}

/// Pure: stable versions only, sorted newest first.
pub(crate) fn list_zig_versions(index_body: &str) -> Vec<String> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(index_body) else {
        return Vec::new();
    };
    let Some(obj) = root.as_object() else {
        return Vec::new();
    };
    let mut out: Vec<String> = obj
        .keys()
        .filter(|k| *k != "master" && !k.contains('-'))
        .cloned()
        .collect();
    out.sort_by(|a, b| common::version_cmp(b, a));
    out
}

/// `.tar.xz` extraction via lzma-rs + tar-rs. Pure-Rust, no liblzma.
fn extract_tar_xz(archive: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(archive).with_context(|| format!("open {}", archive.display()))?;
    let mut reader = std::io::BufReader::new(f);
    let mut decompressed = Vec::new();
    lzma_rs::xz_decompress(&mut reader, &mut decompressed)
        .with_context(|| format!("xz-decompress {}", archive.display()))?;
    let mut tar = tar::Archive::new(std::io::Cursor::new(decompressed));
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    tar.unpack(dest)
        .with_context(|| format!("unpack tar to {}", dest.display()))?;
    Ok(())
}

fn find_zig_top(store_dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let p = e.path();
            if p.join("zig").is_file() {
                return Ok(p);
            }
        }
    }
    bail!(
        "no `zig` binary inside extracted archive at {}",
        store_dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "master": {
        "version": "0.16.0-dev.1234",
        "x86_64-macos": { "tarball": "https://x.test/master.tar.xz", "shasum": "deadbeef", "size": "1" }
      },
      "0.16.0": {
        "x86_64-macos": { "tarball": "https://x.test/0.16.0/macos.tar.xz", "shasum": "abc111", "size": "1" },
        "x86_64-linux": { "tarball": "https://x.test/0.16.0/linux.tar.xz", "shasum": "abc222", "size": "1" }
      },
      "0.15.1": {
        "x86_64-macos": { "tarball": "https://x.test/0.15.1/macos.tar.xz", "shasum": "def111", "size": "1" }
      },
      "0.14.1-rc.1": {
        "x86_64-macos": { "tarball": "https://x.test/rc/macos.tar.xz", "shasum": "rrr", "size": "1" }
      }
    }"#;

    #[test]
    fn picks_correct_asset() {
        let a = pick_zig_asset(SAMPLE, "0.16.0", "x86_64-macos")
            .unwrap()
            .unwrap();
        assert_eq!(a.shasum, "abc111");
        assert!(a.tarball.contains("0.16.0/macos"));
    }

    #[test]
    fn returns_none_when_version_missing() {
        assert!(pick_zig_asset(SAMPLE, "9.9.9", "x86_64-macos")
            .unwrap()
            .is_none());
    }

    #[test]
    fn returns_none_when_triple_missing() {
        assert!(pick_zig_asset(SAMPLE, "0.15.1", "x86_64-linux")
            .unwrap()
            .is_none());
    }

    #[test]
    fn list_filters_master_and_prereleases() {
        let v = list_zig_versions(SAMPLE);
        assert_eq!(v, vec!["0.16.0", "0.15.1"]);
    }
}
