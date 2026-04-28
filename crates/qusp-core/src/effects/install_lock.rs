//! Install-path correctness primitives — Phase 5 外 fix (audit W1, F4).
//!
//! Two issues this module addresses:
//!
//! ### W1 — concurrent install race
//!
//! `qusp install <lang> <version>` extracts a tarball into a content-
//! addressed store dir, then symlinks `data/<lang>/<version>` at it.
//! Two processes running the same install simultaneously (CI matrix,
//! multi-shell user, future `qusp run` racing) would both try to
//! delete + recreate the symlink and extract into the same store
//! dir. The result: partial extracts, dangling symlinks, sha-mismatch
//! crashes from a half-overwritten file.
//!
//! Fix: an advisory `flock` (POSIX `fcntl` / Win32 `LockFileEx`) on a
//! per-install-dir lock file. The first install holds the lock; any
//! subsequent install for the same `<lang>+<version>` blocks until
//! the first releases. Different languages or different versions
//! never collide (different lock files), so legitimate parallel
//! `qusp install` of distinct toolchains stays parallel.
//!
//! ### atomic_symlink_swap
//!
//! Replacing the install-dir symlink as `remove` + `symlink` has a
//! brief window where readers (`qusp run`, `which python`) see
//! ENOENT. POSIX `rename` is atomic even when the destination
//! exists, so we materialise the new symlink at a sibling path
//! (`<install_dir>.qusp-tmp-<pid>`) and `rename` it into place.
//!
//! ### Why these are correctness, not hospitality
//!
//! Both bugs corrupt installs / break readers under concurrency.
//! The user can lose hours of build time to a single misclicked
//! parallel `qusp install`. Hospitality (Phase 5) makes the tool
//! pleasant; this module makes it *correct under load* — a
//! prerequisite for daily-driver dogfood.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use fs2::FileExt;

/// An advisory exclusive file lock held for the lifetime of the
/// guard. Drop releases. Identical-key contenders block on
/// `acquire`; different keys are independent.
pub struct StoreLock {
    file: std::fs::File,
    path: PathBuf,
}

impl StoreLock {
    /// Acquire an exclusive lock keyed on `lock_path`. The path's
    /// parent directory must exist or be creatable; the lock file
    /// itself is created if absent (and not removed on drop, by
    /// design — recreating it on each install would race the lock).
    ///
    /// Blocks until the lock is available. Logs to `tracing::info`
    /// every 5 seconds so a stuck CI is debuggable.
    pub fn acquire(lock_path: &Path) -> Result<Self> {
        if let Some(parent) = lock_path.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path)
            .with_context(|| format!("open install lock {}", lock_path.display()))?;
        let mut announced = false;
        let started = std::time::Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) => {
                    let elapsed = started.elapsed();
                    if !announced && elapsed >= Duration::from_secs(2) {
                        tracing::info!(
                            "waiting for install lock at {} (held by another qusp process)",
                            lock_path.display()
                        );
                        announced = true;
                    } else if announced && elapsed.as_secs() % 5 == 0 {
                        tracing::info!(
                            "still waiting for install lock at {} ({:?})",
                            lock_path.display(),
                            elapsed
                        );
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        Ok(Self {
            file,
            path: lock_path.to_path_buf(),
        })
    }

    /// Path of the lock file (for diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Materialise a new symlink at `link_path` pointing to `target`,
/// replacing any existing symlink atomically (POSIX `rename`).
///
/// On macOS / Linux this guarantees readers (`qusp run`,
/// `which python`) see either the old symlink or the new symlink,
/// never ENOENT, even mid-install.
///
/// On Windows we fall back to `remove + symlink_dir` (Win32 `MoveFileEx`
/// has slightly different semantics for symlinks; an atomic swap exists
/// via `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` but this code path
/// hasn't been exercised on Win, and qusp's primary platform is Unix).
pub fn atomic_symlink_swap(target: &Path, link_path: &Path) -> Result<()> {
    if let Some(parent) = link_path.parent() {
        anyv_core::paths::ensure_dir(parent)?;
    }
    #[cfg(unix)]
    {
        let tmp = link_path.with_extension(format!("qusp-tmp-{}", std::process::id()));
        // Best-effort cleanup of any stale tmp from a prior crashed run.
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(target, &tmp).with_context(|| {
            format!(
                "create temp symlink {} → {}",
                tmp.display(),
                target.display()
            )
        })?;
        std::fs::rename(&tmp, link_path).with_context(|| {
            format!(
                "rename {} → {} (atomic swap)",
                tmp.display(),
                link_path.display()
            )
        })?;
    }
    #[cfg(windows)]
    {
        if link_path.exists() || link_path.is_symlink() {
            let _ = std::fs::remove_file(link_path);
            let _ = std::fs::remove_dir_all(link_path);
        }
        std::os::windows::fs::symlink_dir(target, link_path).with_context(|| {
            format!("symlink {} → {}", link_path.display(), target.display())
        })?;
    }
    Ok(())
}

/// Convenience: derive the install-lock file path for a given
/// install_dir. We use a sibling `<name>.qusp-lock` so the lock file
/// lives in the parent dir (which is `data/<lang>/`) and isn't
/// disturbed by remove_dir_all on the install_dir itself.
pub fn lock_path_for(install_dir: &Path) -> PathBuf {
    let mut p = install_dir.to_path_buf();
    let name = install_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lock".into());
    p.set_file_name(format!("{name}.qusp-lock"));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_path_is_sibling_of_install_dir() {
        let p = lock_path_for(Path::new("/tmp/data/lua/5.4.7"));
        assert_eq!(p, Path::new("/tmp/data/lua/5.4.7.qusp-lock"));
    }

    #[test]
    fn atomic_symlink_swap_creates_and_replaces() {
        let dir = std::env::temp_dir().join(format!(
            "qusp-symswap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        anyv_core::paths::ensure_dir(&dir).unwrap();
        let target_a = dir.join("target_a");
        let target_b = dir.join("target_b");
        anyv_core::paths::ensure_dir(&target_a).unwrap();
        anyv_core::paths::ensure_dir(&target_b).unwrap();
        let link = dir.join("link");

        // First create.
        atomic_symlink_swap(&target_a, &link).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), target_a);

        // Replace.
        atomic_symlink_swap(&target_b, &link).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), target_b);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn store_lock_acquire_release() {
        let lock_path = std::env::temp_dir().join(format!(
            "qusp-storelock-{}-{}.lock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        // First acquire succeeds.
        let g1 = StoreLock::acquire(&lock_path).unwrap();
        // Second acquire would block — exercise release-after-drop:
        drop(g1);
        // Now another acquire on the same path is unblocked.
        let _g2 = StoreLock::acquire(&lock_path).unwrap();
        // (g2 released on drop at end of test.)
        std::fs::remove_file(&lock_path).ok();
    }
}
