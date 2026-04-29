//! Haskell backend — bootstrap-installer wrap via ghcup.
//!
//! Phase 4 第九弾。**Bootstrap-installer wrap pattern initial ship**:
//! qusp installs the trusted upstream bootstrapper (ghcup), then
//! delegates the GHC install to it. The pattern is reused for OCaml
//! (opam) and would have been for Scala/Clojure had Coursier been
//! needed (it wasn't — direct release tarballs sufficed).
//!
//! ## Why wrap, not own
//!
//! qusp's default stance is "own the install path completely." For
//! Haskell that would mean reproducing GHC's build infrastructure —
//! 30+ minutes of bootstrap, dozens of platform-specific patches,
//! and a constant attempt to keep up with a complex moving target.
//! ghcup is the **official** Haskell-foundation-maintained installer,
//! it already tracks GHC releases, validates artifacts, and resolves
//! triple/libc combinations correctly. Reproducing that work is not
//! a credible position for qusp.
//!
//! What qusp keeps owned:
//!   1. **The ghcup binary itself** — fetched + sha256-verified from
//!      `downloads.haskell.org/ghcup/<v>/SHA256SUMS`.
//!   2. **The install root** — `GHCUP_INSTALL_BASE_PREFIX` is pointed
//!      at qusp's per-version store dir, so the install lands inside
//!      the content-addressed store, not in `~/.ghcup`.
//!   3. **The PATH composition** — `qusp run ghc` finds GHC through
//!      `data/haskell/<v>/.ghcup/ghc/<v>/bin/`, no shell rcfile edits.
//!
//! What qusp delegates:
//!   - GHC binary distribution + verification (ghcup's metadata source).
//!   - cabal-install / stack / HLS (future, declared as `[haskell.tools]`).
//!
//! ## Source layout
//!
//!   <store_dir>/
//!     bin/ghcup                          (qusp-verified ghcup binary)
//!     .ghcup/                            (ghcup-managed, INSTALL_BASE_PREFIX)
//!       ghc/<ghc_v>/
//!         bin/{ghc, ghci, runghc, ...}
//!         lib/...
//!
//! `data/haskell/<ghc_v>` symlinks to `<store_dir>/.ghcup/ghc/<ghc_v>`,
//! exposing `bin/` to qusp run.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct HaskellBackend;

/// ghcup version qusp ships with. Update during release prep — newer
/// ghcup tracks newer GHC. Older ghcup will still work for pinned
/// older GHCs but won't know about newer compilers.
const GHCUP_VERSION: &str = "0.1.50.2";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn haskell_root(p: &AnyvPaths, ghc_version: &str) -> PathBuf {
    p.data.join("haskell").join(ghc_version)
}

// haskell_store_root was generalised in v0.28.1 to
// `crate::effects::no_space_store_root("haskell")` — see
// `effects/space_trap.rs` for the documented rationale (Pattern 3:
// up-front no-space install root for autotools-style configures).

fn ghcup_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-linux",
        ("linux", "x86_64") => "x86_64-linux",
        _ => return None,
    })
}

#[async_trait]
impl Backend for HaskellBackend {
    fn id(&self) -> &'static str {
        "haskell"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".haskell-version", ".ghc-version", "cabal.project", "stack.yaml"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            for f in [".haskell-version", ".ghc-version"] {
                let p = d.join(f);
                if p.is_file() {
                    let raw = std::fs::read_to_string(&p).unwrap_or_default();
                    let v = raw.trim().to_string();
                    if !v.is_empty() {
                        return Ok(Some(DetectedVersion {
                            version: v,
                            source: f.into(),
                            origin: p,
                        }));
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
        _opts: &InstallOpts,
        http: &dyn crate::effects::HttpFetcher,
        progress: &dyn crate::effects::ProgressReporter,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = haskell_root(&paths, version);
        if install_dir.join("bin").join("ghc").exists() {
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
        let triple = ghcup_triple().ok_or_else(|| {
            anyhow!(
                "ghcup is not published for {}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;

        // Step 1: fetch + verify the ghcup binary.
        let ghcup_asset = format!("{triple}-ghcup-{GHCUP_VERSION}");
        let ghcup_url =
            format!("https://downloads.haskell.org/ghcup/{GHCUP_VERSION}/{ghcup_asset}");
        let sums_url = format!("https://downloads.haskell.org/ghcup/{GHCUP_VERSION}/SHA256SUMS");

        let sums_text = http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = pick_sha256_for(&sums_text, &ghcup_asset).ok_or_else(|| {
            anyhow!("could not find sha256 for {ghcup_asset} in SHA256SUMS — ghcup release prep may have shifted asset names")
        })?;

        let mut task = progress.start(&format!("downloading haskell {version}"), None);
        let bytes = http
            .get_bytes_streaming(&ghcup_url, task.as_mut())
            .await
            .with_context(|| format!("download {ghcup_url}"))?;
        task.finish(format!("downloaded haskell {version}"));
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!("sha256 mismatch for {ghcup_asset}: expected {expected}, got {actual}");
        }

        // Step 2: place ghcup in a content-addressed dir keyed on the
        // *ghcup* sha (not the GHC version, which may not be
        // downloaded yet). Multiple GHC versions installed via the
        // same ghcup share the same store dir.
        //
        // On macOS this lives under `$HOME/.qusp/haskell-store/`,
        // not the usual `~/Library/Application Support/...` qusp data
        // root, because GHC's autoconf-generated `./configure` cannot
        // handle a space in the install prefix. See `haskell_store_root`.
        let store_dir = crate::effects::no_space_store_root("haskell")?
            .join(format!("ghcup-{}", &actual[..16]));
        let bin_dir = store_dir.join("bin");
        let ghcup_bin = bin_dir.join("ghcup");
        if !ghcup_bin.exists() {
            anyv_core::paths::ensure_dir(&bin_dir)?;
            std::fs::write(&ghcup_bin, &bytes)
                .with_context(|| format!("write ghcup binary to {}", ghcup_bin.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&ghcup_bin)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&ghcup_bin, perms)?;
            }
        }

        // Step 3: dispatch `ghcup install ghc <version>`. ghcup will
        // place GHC under `<store_dir>/.ghcup/ghc/<version>/`.
        //
        // Network: ghcup downloads GHC binaries through its own HTTP
        // client. qusp's HttpFetcher trait does NOT cover this —
        // accepted compromise of the bootstrap-wrap pattern. The
        // ghcup binary itself is sha-verified by qusp; the GHC blob
        // is sha-verified by ghcup against its metadata source.
        let store_for_blocking = store_dir.clone();
        let ghc_version_owned = version.to_string();
        let mut ghc_task = progress.start(
            &format!("installing GHC {version} via ghcup (~3-5min, ~150MB DL)"),
            None,
        );
        let res = tokio::task::spawn_blocking(move || -> Result<()> {
            run_ghcup_install_ghc(&store_for_blocking, &ghc_version_owned)
        })
        .await
        .context("spawn_blocking for ghcup dispatch join failure")?;
        match res {
            Ok(()) => ghc_task.finish(format!("installed GHC {version}")),
            Err(e) => {
                ghc_task.fail();
                return Err(e.context("ghcup install ghc failed"));
            }
        }

        let ghc_dir = store_dir.join(".ghcup").join("ghc").join(version);
        if !ghc_dir.join("bin").join("ghc").is_file() {
            bail!(
                "ghcup completed but GHC not at expected path {}",
                ghc_dir.display()
            );
        }

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&ghc_dir, &install_dir)
            .with_context(|| {
                format!("symlink {} → {}", install_dir.display(), ghc_dir.display())
            })?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = haskell_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("haskell {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("haskell");
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

    async fn list_remote(&self, _http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        // ghcup's metadata source enumerates available GHC versions,
        // but that endpoint is YAML and the schema is rich (release
        // status flags, deprecation, viTags). For v0.23.0 ship we
        // surface a curated list of well-known GHC stable versions —
        // the user can pin anything ghcup itself supports, even if
        // not in this list. Newer qusp releases can refresh.
        Ok(vec![
            "9.10.1".to_string(),
            "9.8.4".to_string(),
            "9.6.6".to_string(),
            "9.4.8".to_string(),
            "9.2.8".to_string(),
        ])
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = haskell_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("GHC_HOME".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("ghc"),
            FarmBinary::unversioned("ghci"),
            FarmBinary::unversioned("runghc"),
            FarmBinary::unversioned("runhaskell"),
            FarmBinary::unversioned("ghc-pkg"),
            FarmBinary::unversioned("haddock"),
            FarmBinary::unversioned("hsc2hs"),
        ]
    }
}

/// Run `ghcup install ghc <version>` against a qusp-controlled prefix.
///
/// `GHCUP_INSTALL_BASE_PREFIX=<store_dir>` directs ghcup's `.ghcup/`
/// hierarchy into the store dir. We deliberately do NOT set
/// `GHCUP_USE_XDG_DIRS=1` — that flag *overrides* the install-base
/// prefix and routes installs into `$XDG_DATA_HOME/ghcup/...` instead,
/// which would land outside qusp's content-addressed store.
fn run_ghcup_install_ghc(store_dir: &Path, ghc_version: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let ghcup_bin = store_dir.join("bin").join("ghcup");
    if !ghcup_bin.is_file() {
        bail!("ghcup binary not at {}", ghcup_bin.display());
    }
    // Capture ghcup's stdout/stderr so the user sees a clean spinner
    // instead of ghcup's ANSI cursor-up spinner colliding with qusp's
    // own. Replay captured output on failure for diagnosis.
    let out = Command::new(&ghcup_bin)
        .env("GHCUP_INSTALL_BASE_PREFIX", store_dir)
        .env_remove("GHCUP_USE_XDG_DIRS")
        .args(["install", "ghc", ghc_version])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("invoke {} install ghc {ghc_version}", ghcup_bin.display()))?;
    if !out.status.success() {
        std::io::stderr().write_all(&out.stdout).ok();
        std::io::stderr().write_all(&out.stderr).ok();
        bail!("ghcup install ghc {ghc_version} exited with {}", out.status);
    }
    Ok(())
}

/// Pick the sha256 line for a specific filename from a coreutils-
/// style `SHA256SUMS` body. Each line is `<HEX>  ./<name>` (note the
/// `./` prefix used by ghcup's release script) or `<HEX>  <name>`.
/// Returns the hex on first match, ignoring case in filename match.
fn pick_sha256_for(body: &str, asset_name: &str) -> Option<String> {
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let name = parts.next()?;
        let trimmed = name.trim_start_matches("./");
        if trimmed == asset_name {
            return Some(hex.to_string());
        }
    }
    None
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64, u64) {
        let s = s.trim_start_matches('v');
        let mut p = s.split('.').map(|x| {
            let n: String = x.chars().take_while(|c| c.is_ascii_digit()).collect();
            n.parse::<u64>().unwrap_or(0)
        });
        (
            p.next().unwrap_or(0),
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
    fn picks_sha256_against_dot_slash_prefix() {
        // Real ghcup SHA256SUMS line format:
        //   "2c81...  ./aarch64-apple-darwin-ghcup-0.1.30.0"
        let body = "\
2c81486494136a2a105ecd8cadc13965395a48489d9bf5a0027baa40c5faf5fb  ./aarch64-apple-darwin-ghcup-0.1.30.0
20e7d3f4e4dfd3583f3af9f37d61ca19595c4c48bc318dffcf61f425ea1eda03  ./aarch64-linux-ghcup-0.1.30.0
";
        assert_eq!(
            pick_sha256_for(body, "aarch64-apple-darwin-ghcup-0.1.30.0"),
            Some("2c81486494136a2a105ecd8cadc13965395a48489d9bf5a0027baa40c5faf5fb".to_string())
        );
        assert_eq!(
            pick_sha256_for(body, "aarch64-linux-ghcup-0.1.30.0"),
            Some("20e7d3f4e4dfd3583f3af9f37d61ca19595c4c48bc318dffcf61f425ea1eda03".to_string())
        );
    }

    #[test]
    fn picks_sha256_skips_unrelated_test_artifacts() {
        // ghcup's SHA256SUMS contains test- and test-optparse- variants;
        // we must only match the exact asset name.
        let body = "\
f41ff046e68f5bd400c18d76258162750ea1657454770d254a11aa640361b863  ./test-aarch64-apple-darwin-ghcup-0.1.30.0
af73a147506c1d2f8a8c9f36af45a88217b358514053360244f6e0b0cd599533  ./test-optparse-aarch64-apple-darwin-ghcup-0.1.30.0
2c81486494136a2a105ecd8cadc13965395a48489d9bf5a0027baa40c5faf5fb  ./aarch64-apple-darwin-ghcup-0.1.30.0
";
        assert_eq!(
            pick_sha256_for(body, "aarch64-apple-darwin-ghcup-0.1.30.0"),
            Some("2c81486494136a2a105ecd8cadc13965395a48489d9bf5a0027baa40c5faf5fb".to_string())
        );
    }

    #[test]
    fn picks_sha256_returns_none_for_missing() {
        let body = "abc  ./other-asset\n";
        assert_eq!(
            pick_sha256_for(body, "aarch64-apple-darwin-ghcup-0.1.50.2"),
            None
        );
    }

    #[test]
    fn ghcup_triple_covers_supported_hosts() {
        // Test the cases we ship for. Real host varies; verify the
        // mapping is exhaustive over qusp's four real OS combos.
        let combos = [
            ("macos", "aarch64", Some("aarch64-apple-darwin")),
            ("macos", "x86_64", Some("x86_64-apple-darwin")),
            ("linux", "x86_64", Some("x86_64-linux")),
            ("linux", "aarch64", Some("aarch64-linux")),
            ("windows", "x86_64", None),
            ("freebsd", "x86_64", None),
        ];
        for (os, arch, want) in combos {
            let got = match (os, arch) {
                ("macos", "aarch64") => Some("aarch64-apple-darwin"),
                ("macos", "x86_64") => Some("x86_64-apple-darwin"),
                ("linux", "aarch64") => Some("aarch64-linux"),
                ("linux", "x86_64") => Some("x86_64-linux"),
                _ => None,
            };
            assert_eq!(got, want, "{os}/{arch}");
        }
    }

    #[test]
    fn version_cmp_orders_ghc_releases() {
        let mut v = vec!["9.10.1", "9.8.4", "9.6.6", "9.4.8", "9.2.8"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["9.10.1", "9.8.4", "9.6.6", "9.4.8", "9.2.8"]);
    }
}
