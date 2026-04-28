//! Helpers for sidestepping the macOS "Application Support" space trap.
//!
//! qusp's data root on macOS resolves to
//! `~/Library/Application Support/dev.O6lvl4.qusp/` — the literal space
//! in `Application Support` consistently breaks third-party tooling
//! that expands paths unquoted. Across Phase 4 we hit it four times:
//!
//! 1. **Groovy** v0.18.0 — upstream `bin/startGroovy` appends
//!    `-Xdock:icon=$GROOVY_HOME/lib/groovy.icns` to `$JAVA_OPTS`, then
//!    expands `$JAVA_OPTS` *unquoted* in `exec java`.
//! 2. **Clojure** v0.21.0 — qusp's own sed substitution into
//!    `install_dir=PREFIX` (the launcher's bare assignment).
//! 3. **Lua** v0.22.0 — upstream Makefile's `install:` recipe
//!    word-splits `$(INSTALL_BIN)` into `install -p ...` args.
//! 4. **Haskell** v0.23.0 — GHC's autoconf-generated `./configure`
//!    word-splits `--prefix=...` at install time.
//!
//! Three escalating mitigation patterns emerged. This module exposes
//! the helpers each one needs, so future source-build / wrap backends
//! (OCaml, PHP, R, Erlang/OTP) can use them without rebuilding the
//! reasoning per backend.
//!
//! ## Pattern 1: in-place launcher patch
//!
//! For simple shell launchers, replace the offending injection by
//! single-quote-escaping the substituted value. See
//! [`shell_single_quote`].
//!
//! ## Pattern 2: stage-and-move
//!
//! For Makefile/script installs that don't sed-friendly fix, run the
//! install against a no-space staging dir (under
//! `std::env::temp_dir()` which on macOS is `/var/folders/...`,
//! guaranteed space-free), then `rename` the result into the qusp
//! store. See [`mktemp_no_space`] and [`copy_tree`].
//!
//! ## Pattern 3: up-front no-space store
//!
//! For autotools-style configures that bake the prefix into wrapper
//! scripts and pkg-config files, neither in-place patching nor
//! post-install relocation works. The install must land at a
//! no-space path *up-front*. See [`no_space_store_root`].

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

// ─── Pattern 1: shell-quote escape ──────────────────────────────────

/// Wrap `s` in shell single-quotes, escaping any embedded single
/// quotes via the standard `'\''` close-escape-reopen idiom. The
/// result is a single token safe to splice into a POSIX-shell
/// assignment like `var='<value>'` even when `s` contains spaces or
/// metacharacters.
///
/// Use this when patching upstream launcher scripts (Pattern 1) — the
/// canonical example is Clojure's `install_dir=PREFIX` line, where
/// `PREFIX` is sed-substituted to a path that may contain spaces.
pub fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

// ─── Pattern 2: no-space staging dir ────────────────────────────────

/// Create a fresh temp dir under `std::env::temp_dir()`. On macOS this
/// resolves to `/var/folders/...` — guaranteed space-free regardless
/// of the user's `$HOME` shape. Concurrent qusp installs disambiguate
/// via pid + atomic counter so two parallel `qusp install lua 5.4.7`
/// don't collide on the same staging path.
///
/// The directory is the caller's to clean up (or leak — `temp_dir()`
/// is auto-purged by the OS eventually).
pub fn mktemp_no_space(label: &str) -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir();
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("{label}-{pid}-{n}-{nanos}"));
    anyv_core::paths::ensure_dir(&dir)?;
    Ok(dir)
}

/// Recursive directory copy preserving file modes and symlinks. Used
/// as the cross-filesystem fallback when `rename` can't move a tree
/// from `mktemp_no_space()`'s `/var/folders/...` mount into the qusp
/// store under `~/Library/...`.
///
/// Symlinks are recreated (not followed) on Unix; on Windows they're
/// silently skipped to avoid the developer-mode-required Win32
/// `CreateSymbolicLink` privilege check.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    anyv_core::paths::ensure_dir(dst)?;
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("read_dir {}", src.display()))?
    {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_tree(&from, &to)?;
        } else if ty.is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(&from)?;
                std::os::unix::fs::symlink(&target, &to).ok();
            }
            // Windows: skipped — see doc above.
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copy {} → {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

// ─── Pattern 3: up-front no-space install root ──────────────────────

/// Resolve a no-space install root for backends whose upstream
/// installer bakes the prefix path into wrapper scripts / pkg-config
/// files (autotools `--prefix`, etc.) — Pattern 3.
///
/// On macOS, `~/Library/Application Support/dev.O6lvl4.qusp/...`
/// contains a space and is unsuitable. We divert to
/// `$HOME/.qusp/<label>-store/` — qusp-namespaced, persistent (not
/// macOS Caches which is OS-purgeable), no-space by construction.
/// If `$HOME` itself contains a space (highly unusual for technical
/// macOS users), we bail with a clear error rather than producing a
/// half-broken install.
///
/// On Linux/BSD, qusp's normal data path is already space-free, so we
/// fall back to the regular `paths.store()` directory.
///
/// `label` is included in the dir name so concurrent uses of this
/// helper from different backends don't share a flat namespace.
/// Conventionally: `"haskell"`, `"ocaml"`, `"erlang"` etc.
pub fn no_space_store_root(label: &str) -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| anyhow!("HOME not set; required for no-space store on macOS"))?;
        let path = PathBuf::from(home)
            .join(".qusp")
            .join(format!("{label}-store"));
        if path.to_string_lossy().contains(' ') {
            bail!(
                "no-space store path {} contains a space — \
                 this backend requires a space-free install prefix \
                 (autotools/wrapper-baked path constraint). Move your \
                 $HOME or set a space-free path.",
                path.display()
            );
        }
        anyv_core::paths::ensure_dir(&path)?;
        Ok(path)
    } else {
        // Linux / BSD: ~/.local/share/qusp/store/ is already space-free.
        let p = anyv_core::Paths::discover("qusp")?;
        let path = p.store();
        anyv_core::paths::ensure_dir(&path)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_single_quote_handles_application_support_path() {
        // Clojure's actual launcher target.
        assert_eq!(
            shell_single_quote(
                "/Users/o6lvl4/Library/Application Support/dev.O6lvl4.qusp/store/abc/prefix/lib/clojure"
            ),
            "'/Users/o6lvl4/Library/Application Support/dev.O6lvl4.qusp/store/abc/prefix/lib/clojure'"
        );
    }

    #[test]
    fn shell_single_quote_escapes_embedded_apostrophe() {
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn shell_single_quote_handles_empty() {
        assert_eq!(shell_single_quote(""), "''");
    }

    #[test]
    fn mktemp_no_space_dirs_are_unique_and_space_free() {
        let a = mktemp_no_space("qusp-test").unwrap();
        let b = mktemp_no_space("qusp-test").unwrap();
        assert_ne!(a, b, "concurrent calls must not collide");
        assert!(
            !a.to_string_lossy().contains(' '),
            "must be space-free: {}",
            a.display()
        );
        assert!(
            !b.to_string_lossy().contains(' '),
            "must be space-free: {}",
            b.display()
        );
        std::fs::remove_dir_all(&a).ok();
        std::fs::remove_dir_all(&b).ok();
    }

    #[test]
    fn copy_tree_preserves_file_contents() {
        let src = mktemp_no_space("qusp-copytree-src").unwrap();
        let dst = std::env::temp_dir().join(format!(
            "qusp-copytree-dst-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        anyv_core::paths::ensure_dir(&src.join("sub")).unwrap();
        std::fs::write(src.join("sub/b.txt"), "world").unwrap();
        copy_tree(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(
            std::fs::read_to_string(dst.join("sub/b.txt")).unwrap(),
            "world"
        );
        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }
}
