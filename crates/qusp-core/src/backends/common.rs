//! Shared helpers for backend implementations.
//!
//! Every qusp backend repeats a handful of operations verbatim:
//! version comparison, installed-version listing, uninstall, SHA256
//! download-verify, and the cache→store→extract pipeline. This module
//! extracts those into reusable functions so backends only contain
//! language-specific logic.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use sha2::Digest;

use crate::backend::InstallReport;
use bytes::Bytes;

use crate::effects::HttpFetcher;

// ─── Paths ──────────────────────────────────────────────────────────

pub fn qusp_paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

pub fn lang_root(paths: &AnyvPaths, lang: &str, version: &str) -> PathBuf {
    paths.data.join(lang).join(version)
}

// ─── OS / arch ──────────────────────────────────────────────────────

/// Returns `(os, arch)` using Rust's `std::env::consts`.
/// Backends map these to their upstream's naming convention.
pub fn os_arch() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

// ─── Version comparison ─────────────────────────────────────────────

/// Semver-ish comparison: split on `.`, compare numerically.
/// Non-numeric segments compare as 0.
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.strip_prefix('v').unwrap_or(s);
        let s = s.split_whitespace().next().unwrap_or(s);
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}

// ─── Install guard ──────────────────────────────────────────────────

/// Check if a toolchain is already installed by looking for a marker
/// file (e.g. `bin/rustc`, `bin/node`, `zig`).
/// Returns `Some(InstallReport)` if present, `None` if install needed.
pub fn check_already_installed(
    install_dir: &Path,
    marker: &str,
    version: &str,
) -> Option<InstallReport> {
    if install_dir.join(marker).exists() {
        Some(InstallReport {
            version: version.to_string(),
            install_dir: install_dir.to_path_buf(),
            already_present: true,
        })
    } else {
        None
    }
}

/// Acquire the install lock for a given install_dir.
pub fn acquire_install_lock(
    install_dir: &Path,
) -> Result<crate::effects::StoreLock> {
    crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(install_dir))
}

// ─── list_installed / uninstall ─────────────────────────────────────

/// List installed versions for a language, sorted newest first.
pub fn list_installed_versions(lang: &str) -> Result<Vec<String>> {
    let paths = qusp_paths()?;
    let dir = paths.data.join(lang);
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

/// Uninstall a single version for a language.
pub fn uninstall_version(lang: &str, version: &str) -> Result<()> {
    let paths = qusp_paths()?;
    let dir = lang_root(&paths, lang, version);
    if !dir.exists() && !dir.is_symlink() {
        bail!("{lang} {version} is not installed via qusp");
    }
    std::fs::remove_file(&dir)
        .or_else(|_| std::fs::remove_dir_all(&dir))
        .with_context(|| format!("remove {}", dir.display()))?;
    Ok(())
}

// ─── Download + SHA256 verify ───────────────────────────────────────

/// Download bytes from `url`, verify against `expected_sha256`.
/// Uses streaming download with progress reporting.
pub async fn download_and_verify(
    http: &dyn HttpFetcher,
    url: &str,
    expected_sha: &str,
    progress: &dyn crate::effects::ProgressReporter,
    label: &str,
) -> Result<Bytes> {
    let mut task = progress.start(&format!("downloading {label}"), None);
    let bytes = http
        .get_bytes_streaming(url, task.as_mut())
        .await
        .with_context(|| format!("download {url}"))?;
    task.finish(format!("downloaded {label}"));

    verify_sha256(&bytes, expected_sha, url)?;
    Ok(bytes)
}

/// Verify SHA256 of bytes against expected hex digest.
pub fn verify_sha256(bytes: &[u8], expected: &str, label: &str) -> Result<()> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let actual = hex::encode(hasher.finalize());
    if !expected.eq_ignore_ascii_case(&actual) {
        bail!("sha256 mismatch for {label}: expected {expected}, got {actual}");
    }
    Ok(())
}

// ─── Cache → store → extract pipeline ──────────────────────────────

/// Write bytes to cache, extract into content-addressed store dir,
/// clean up the cache file. Returns the store directory path.
/// Uses `anyv_core::extract::extract_archive` which handles
/// `.tar.gz`, `.tar.xz`, and `.zip` based on extension.
pub fn stage_to_store(
    paths: &AnyvPaths,
    bytes: impl AsRef<[u8]>,
    sha_hex: &str,
    cache_name: &str,
) -> Result<PathBuf> {
    let cache_path = paths.cache.join(cache_name);
    anyv_core::paths::ensure_dir(&paths.cache)?;
    std::fs::write(&cache_path, bytes.as_ref())?;

    let store_dir = paths.store().join(&sha_hex[..16]);
    if store_dir.exists() {
        std::fs::remove_dir_all(&store_dir).ok();
    }
    anyv_core::paths::ensure_dir(&store_dir)?;
    anyv_core::extract::extract_archive(&cache_path, &store_dir)?;
    let _ = std::fs::remove_file(&cache_path);

    Ok(store_dir)
}

// ─── Symlink finalization ───────────────────────────────────────────

/// Create parent dir and atomic-swap symlink from `source` to `install_dir`.
pub fn finalize_install(source: &Path, install_dir: &Path) -> Result<()> {
    if let Some(parent) = install_dir.parent() {
        anyv_core::paths::ensure_dir(parent)?;
    }
    crate::effects::atomic_symlink_swap(source, install_dir).with_context(|| {
        format!(
            "symlink {} → {}",
            install_dir.display(),
            source.display()
        )
    })
}
