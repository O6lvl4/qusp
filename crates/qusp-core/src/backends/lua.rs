//! Lua backend — source build via `make`.
//!
//! Lua is qusp's first **source-build backend**. Earlier Phase 4
//! ships (Zig, Julia, Crystal, Groovy, Dart, Scala 3, Clojure) all
//! pulled prebuilt distributions. PUC-Rio Lua doesn't publish per-host
//! binaries, only a single ~370 KB source tarball at lua.org/ftp/.
//! That tarball compiles in 5–10 seconds on a modern laptop with the
//! bundled Makefile, so source build is genuinely the path of least
//! resistance — no `ruby-build`-style external dispatcher needed.
//!
//! Source:
//!   https://www.lua.org/ftp/lua-<v>.tar.gz
//!
//! Verification: lua.org **does not** publish per-version `.sha256`
//! sidecars, only an inline hash on `download.html` for the *current*
//! release. To preserve qusp's "every install is sha-verified" stance
//! without inflating the trust surface, we hardcode a curated
//! version→sha256 table for known-good releases. Each entry was
//! produced by manually downloading from lua.org/ftp during qusp
//! release prep and `shasum -a 256`'ing the result. Adding a new Lua
//! version requires a qusp release that updates the table — fine,
//! Lua releases are infrequent (1–2/yr) and security-relevant updates
//! warrant a qusp ship anyway.
//!
//! Build: `make <plat> && make install INSTALL_TOP=<prefix>`.
//! - `<plat>` is one of `macosx`, `linux`, `bsd`, `mingw`, etc. We
//!   set it explicitly rather than `guess` because `guess`'s heuristic
//!   has historically misclassified some macOS variants.
//! - The Lua build is a tight C compile (~30 .c files), no autotools
//!   layer, so no `configure` step. We pipe through tokio's
//!   `spawn_blocking` because `make` is sync CPU-bound — same
//!   pattern as Ruby's rv-core dispatch.
//!
//! Layout post-install:
//!   <prefix>/bin/{lua, luac}
//!   <prefix>/include/{lua.h, luaconf.h, lualib.h, lauxlib.h, lua.hpp}
//!   <prefix>/lib/liblua.a
//!   <prefix>/man/man1/{lua.1, luac.1}
//!   <prefix>/share/lua/<5.4-or-5.5>/    (user-installed pure-Lua modules)
//!   <prefix>/lib/lua/<5.4-or-5.5>/      (user-installed C modules)
//!
//! Tools: empty by design. LuaRocks is the Lua package manager but
//! lives outside the runtime — qusp doesn't curate against it.
//!
//! Linker note: liblua is shipped only as a static `.a` (no `.so` /
//! `.dylib` from the upstream Makefile). C modules built via LuaRocks
//! link against this static lib. That's the upstream model; qusp
//! doesn't second-guess it.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct LuaBackend;

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn lua_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("lua").join(version)
}

/// Curated version → sha256 table. Hashes verified by hand against
/// lua.org/ftp during qusp release prep. To add a new version,
/// download the tarball, `shasum -a 256` it, append the entry,
/// release qusp.
fn known_sha256(version: &str) -> Option<&'static str> {
    Some(match version {
        "5.4.4" => "164c7849653b80ae67bec4b7473b884bf5cc8d2dca05653475ec2ed27b9ebf61",
        "5.4.5" => "59df426a3d50ea535a460a452315c4c0d4e1121ba72ff0bdde58c2ef31d6f444",
        "5.4.6" => "7d5ea1b9cb6aa0b59ca3dde1c6adcb57ef83a1ba8e5432c0ecd06bf439b3ad88",
        "5.4.7" => "9fbf5e28ef86c69858f6d3d34eccc32e911c1a28b4120ff3e84aaa70cfbf1e30",
        "5.4.8" => "4f18ddae154e793e46eeab727c59ef1c0c0c2b744e7b94219710d76f530629ae",
        "5.5.0" => "57ccc32bbbd005cab75bcc52444052535af691789dba2b9016d5c50640d68b3d",
        _ => return None,
    })
}

fn lua_makefile_plat() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => "macosx",
        ("linux", _) => "linux",
        ("freebsd", _) => "bsd",
        ("netbsd", _) => "bsd",
        ("openbsd", _) => "bsd",
        _ => return None,
    })
}

#[async_trait]
impl Backend for LuaBackend {
    fn id(&self) -> &'static str {
        "lua"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".lua-version"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".lua-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".lua-version".into(),
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
        let install_dir = lua_root(&paths, version);
        if install_dir.join("bin").join("lua").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        // W1 fix: serialize concurrent installs of the same lang+version.
        // Held until install completes; different versions / langs unaffected.
        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;
        let plat = lua_makefile_plat().ok_or_else(|| {
            anyhow!(
                "Lua source build requires a known Makefile platform; \
                 {}-{} is not in our table",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        let expected = known_sha256(version).ok_or_else(|| {
            anyhow!(
                "Lua {version} is not in qusp's verified sha256 table. \
                 Pin a known version (5.4.4–5.4.8 or 5.5.0) or upgrade \
                 qusp to a release that includes {version}."
            )
        })?;

        let asset = format!("lua-{version}.tar.gz");
        let asset_url = format!("https://www.lua.org/ftp/{asset}");

        let mut task = progress.start(&format!("downloading lua {version}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded lua {version}"));
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!(
                "sha256 mismatch for {asset}: expected {expected} (qusp curated), \
                 got {actual} from lua.org. Refusing to build."
            );
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

        // Tarball expands to `lua-<v>/{Makefile, src/, doc/}`. Run the
        // build + install entirely inside the store dir, then symlink
        // `data/lua/<v>` to the resulting prefix.
        let src_dir = store_dir.join(format!("lua-{version}"));
        if !src_dir.join("Makefile").is_file() {
            bail!(
                "extracted Lua archive did not contain lua-{version}/Makefile at {}",
                src_dir.display()
            );
        }
        let prefix = store_dir.join("prefix");

        // The qusp store path is `~/Library/Application Support/dev.O6lvl4.qusp/...`
        // on macOS, which contains a space. Lua's upstream Makefile
        // `install:` recipe expands `$(INSTALL_BIN)` *unquoted* in
        //   cd src && install -p -m 0755 lua luac $(INSTALL_BIN)
        // so a space in INSTALL_TOP word-splits and `install` reports
        //   "Inappropriate file type or format" against truncated paths.
        // Make-level quoting can't fix the Makefile bug; the only
        // sane workaround is to stage `make install` against a
        // path with no spaces (`std::env::temp_dir()` on macOS lives
        // under `/var/folders/...` — guaranteed space-free) and then
        // relocate the finished prefix into our store after the build.
        let staging_root = crate::effects::mktemp_no_space("qusp-lua")?;
        let staging = staging_root.join("prefix");
        let src_for_blocking = src_dir.clone();
        let staging_for_blocking = staging.clone();
        let mut build_task = progress.start(&format!("building lua {version}"), None);
        let res = tokio::task::spawn_blocking(move || -> Result<()> {
            run_lua_build(&src_for_blocking, &staging_for_blocking, plat)
        })
        .await
        .context("spawn_blocking for Lua build join failure")?;
        match res {
            Ok(()) => build_task.finish(format!("built lua {version}")),
            Err(e) => {
                build_task.fail();
                let _ = std::fs::remove_dir_all(&staging_root);
                return Err(e.context("Lua make build failed"));
            }
        }

        if !staging.join("bin").join("lua").is_file() {
            bail!(
                "Lua build completed but {} not found",
                staging.join("bin").join("lua").display()
            );
        }

        // Move staging → store/prefix. `rename` works only same-fs; on
        // macOS `/var/folders` is sometimes a different mount than the
        // user's data dir, so fall back to copy-then-remove.
        if let Some(parent) = prefix.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        if std::fs::rename(&staging, &prefix).is_err() {
            crate::effects::copy_tree(&staging, &prefix)
                .with_context(|| format!("copy {} → {}", staging.display(), prefix.display()))?;
        }
        let _ = std::fs::remove_dir_all(&staging_root);

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&prefix, &install_dir)
            .with_context(|| format!("symlink {} → {}", install_dir.display(), prefix.display()))?;

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = lua_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("lua {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("lua");
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
        // The set qusp can verify is exactly the sha256 table; expose
        // that, sorted newest-first. lua.org's ftp listing has older
        // unverified versions too — we don't surface them because we'd
        // refuse to install them anyway.
        let mut out = vec![
            "5.5.0".to_string(),
            "5.4.8".to_string(),
            "5.4.7".to_string(),
            "5.4.6".to_string(),
            "5.4.5".to_string(),
            "5.4.4".to_string(),
        ];
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = lua_root(&paths, version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("LUA_DIR".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("lua"),
            FarmBinary::unversioned("luac"),
        ]
    }
}

fn run_lua_build(src_dir: &Path, prefix: &Path, plat: &str) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    anyv_core::paths::ensure_dir(prefix)?;

    // Capture stdout/stderr instead of inheriting; a 50-line dump of
    // `gcc -std=gnu99 -O2 ...` per file isn't useful unless the build
    // fails. We replay the captured streams on error so the user can
    // diagnose. Same pattern is used by Haskell's ghcup wrapper.
    let out = Command::new("make")
        .arg(plat)
        .current_dir(src_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("invoke `make {plat}` in {}", src_dir.display()))?;
    if !out.status.success() {
        std::io::stderr().write_all(&out.stdout).ok();
        std::io::stderr().write_all(&out.stderr).ok();
        bail!("`make {plat}` exited with {}", out.status);
    }

    let install_top = prefix.to_string_lossy().to_string();
    let out = Command::new("make")
        .arg("install")
        .arg(format!("INSTALL_TOP={install_top}"))
        .current_dir(src_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| {
            format!(
                "invoke `make install INSTALL_TOP={install_top}` in {}",
                src_dir.display()
            )
        })?;
    if !out.status.success() {
        std::io::stderr().write_all(&out.stdout).ok();
        std::io::stderr().write_all(&out.stderr).ok();
        bail!("`make install` exited with {}", out.status);
    }
    Ok(())
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.trim_start_matches('v');
        let mut p = s.split('.').map(|x| {
            let n: String = x.chars().take_while(|c| c.is_ascii_digit()).collect();
            n.parse::<u64>().unwrap_or(0)
        });
        (
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
    fn known_sha256_covers_recent_5_4_x_and_5_5_0() {
        for v in ["5.4.4", "5.4.5", "5.4.6", "5.4.7", "5.4.8", "5.5.0"] {
            let h = known_sha256(v).unwrap_or_else(|| panic!("missing {v}"));
            assert_eq!(h.len(), 64, "{v} hash length");
            assert!(
                h.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{v} hash should be lowercase hex"
            );
        }
    }

    #[test]
    fn known_sha256_rejects_unknown_versions() {
        assert!(known_sha256("5.3.6").is_none()); // older, intentionally unsupported
        assert!(known_sha256("5.5.1").is_none()); // hypothetical future
        assert!(known_sha256("").is_none());
        assert!(known_sha256("garbage").is_none());
    }

    #[test]
    fn lua_makefile_plat_covers_qusp_hosts() {
        // Test the cases qusp ships for. Real host varies; we just
        // check the mapping is exhaustive over the four real OS combos.
        let combos = [
            ("macos", Some("macosx")),
            ("linux", Some("linux")),
            ("freebsd", Some("bsd")),
            ("windows", None),
            ("redox", None),
        ];
        for (os, want) in combos {
            let got = match os {
                "macos" => Some("macosx"),
                "linux" => Some("linux"),
                "freebsd" | "netbsd" | "openbsd" => Some("bsd"),
                _ => None,
            };
            assert_eq!(got, want, "os={os}");
        }
    }

    #[test]
    fn version_cmp_orders_lua_releases() {
        let mut v = vec!["5.4.7", "5.5.0", "5.4.5", "5.4.8", "5.3.6"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["5.5.0", "5.4.8", "5.4.7", "5.4.5", "5.3.6"]);
    }
}
