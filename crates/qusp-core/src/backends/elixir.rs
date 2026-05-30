//! Elixir backend — prebuilt distribution from `elixir-lang/elixir`.
//!
//! Elixir compiles to platform-independent BEAM bytecode, so its
//! official precompiled release zips are not arch-specific — but they
//! ARE keyed to an OTP major version (the bytecode chunk format and
//! stdlib bindings track the Erlang/OTP release). qusp therefore:
//!
//!   1. Requires an Erlang install (`requires() -> ["erlang"]`).
//!   2. Detects the newest installed OTP major and downloads the
//!      matching `elixir-otp-<major>.zip`.
//!
//! ## Release / asset layout
//!
//!   tag:    `v<version>`              (e.g. `v1.18.4`)
//!   asset:  `elixir-otp-26.zip`
//!           `elixir-otp-27.zip`
//!           `elixir-otp-28.zip`       (one per supported OTP major)
//!           `elixir-otp-<n>.zip.sha256sum`   (per-asset sidecar)
//!
//! Verification uses the per-asset `.sha256sum` sidecar (`<sha256>
//! <filename>`), the same shape gleam uses — upstream publishes those
//! plus a `.sigstore` bundle, but no aggregate `SHA512SUMS`.
//!
//! The zip is FLAT: `bin/{elixir,elixirc,iex,mix}` (shell scripts) and
//! `lib/` at the root. The stored / displayed version is the tag with
//! the leading `v` stripped.
//!
//! ## Run-time PATH (the load-bearing detail)
//!
//! `mix`, `iex`, and `elixir` are shell scripts that shell out to
//! `erl`/`escript` at run time. `build_run_env` therefore prepends
//! BOTH `elixir_root/bin` and the newest installed `erlang_root/bin`,
//! so the Elixir launchers can find the Erlang runtime.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use sha2::Digest;

use crate::backend::*;

pub struct ElixirBackend;

const REPO: &str = "elixir-lang/elixir";

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn elixir_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("elixir").join(strip_v(version))
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

/// Newest installed Erlang version directory, or `None` if Erlang is
/// not installed via qusp. Skips lock files.
fn newest_installed_erlang(p: &AnyvPaths) -> Option<String> {
    let dir = p.data.join("erlang");
    if !dir.exists() {
        return None;
    }
    let mut versions: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".qusp-lock"))
        .collect();
    versions.sort_by(|a, b| version_cmp(b, a));
    versions.into_iter().next()
}

/// The OTP major (first dotted component) of a version string.
fn otp_major(version: &str) -> Option<u64> {
    version
        .split('.')
        .next()?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

#[async_trait]
impl Backend for ElixirBackend {
    fn id(&self) -> &'static str {
        "elixir"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["mix.exs"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }
    fn requires(&self) -> &[&'static str] {
        &["erlang"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".elixir-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_v(&v).to_string(),
                        source: ".elixir-version".into(),
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

        let paths = paths()?;
        paths.ensure_dirs()?;
        let v_strip = strip_v(version).to_string();
        let install_dir = elixir_root(&paths, version);
        if install_dir.join("bin").join("elixir").exists() {
            return Ok(InstallReport {
                version: v_strip,
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;

        // Resolve the OTP major from the newest installed Erlang.
        // `requires()` is declared but not auto-consumed by the
        // orchestrator, so enforce the dependency here.
        let erlang_version = newest_installed_erlang(&paths)
            .ok_or_else(|| anyhow!("Elixir requires Erlang. Run `qusp install erlang` first."))?;
        let major = otp_major(&erlang_version).ok_or_else(|| {
            anyhow!("could not parse OTP major from installed erlang `{erlang_version}`")
        })?;

        let tag = format!("v{v_strip}");
        let asset = format!("elixir-otp-{major}.zip");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sha_url = format!("{asset_url}.sha256sum");

        // Verify against the per-asset `.sha256sum` sidecar. A 404 here
        // means this Elixir release ships no zip for the installed OTP
        // major — surface the majors it *does* ship.
        let sha_text = match http.get_text(&sha_url).await {
            Ok(t) => t,
            Err(e) => {
                let avail = available_otp_majors(http, &tag).await.unwrap_or_default();
                if avail.is_empty() {
                    return Err(e)
                        .with_context(|| format!("fetch {sha_url}"))
                        .with_context(|| {
                            format!("Elixir {v_strip} appears to ship no elixir-otp-*.zip assets")
                        });
                }
                bail!(
                    "Elixir {v_strip} ships no {asset} for the installed OTP major {major}. \
                     Published OTP majors: {}. Install a matching Erlang/OTP, or pick an \
                     Elixir version that ships otp-{major}.",
                    avail.join(", ")
                );
            }
        };
        let expected = sha_text
            .split_whitespace()
            .next()
            .ok_or_else(|| anyhow!("empty .sha256sum for {asset}"))?
            .to_string();

        let mut task = progress.start(&format!("downloading elixir {v_strip}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded elixir {v_strip}"));

        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!("sha256 mismatch for {asset}: expected {expected}, got {actual}");
        }

        let cache_path = paths.cache.join(&asset);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        // Key the store dir on the sha512 prefix.
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        // Elixir zip is flat: bin/ and lib/ at the root.
        let bin_dir = store_dir.join("bin");
        let elixir_bin = bin_dir.join("elixir");
        if !elixir_bin.is_file() {
            bail!(
                "extracted elixir archive did not contain bin/elixir under {}",
                store_dir.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["elixir", "elixirc", "iex", "mix"] {
                let p = bin_dir.join(name);
                if let Ok(meta) = std::fs::metadata(&p) {
                    let mut perms = meta.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    std::fs::set_permissions(&p, perms).ok();
                }
            }
        }

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
            version: v_strip,
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = elixir_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("elixir {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("elixir");
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
            serde_json::from_str(&body).context("parse elixir-lang/elixir release index")?;
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
        _name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!("Elixir uses `mix` for Hex deps; qusp doesn't curate an Elixir tool registry.")
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = elixir_root(&paths, version);
        let mut path_prepend = vec![root.join("bin")];
        // Elixir launchers shell out to erl/escript — expose the newest
        // installed Erlang's bin/ too.
        if let Some(erl_v) = newest_installed_erlang(&paths) {
            path_prepend.push(paths.data.join("erlang").join(erl_v).join("bin"));
        }
        Ok(RunEnv {
            path_prepend,
            env: std::collections::BTreeMap::new(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("elixir"),
            FarmBinary::unversioned("elixirc"),
            FarmBinary::unversioned("iex"),
            FarmBinary::unversioned("mix"),
        ]
    }
}

/// The OTP majors an Elixir release publishes a zip for, gathered from
/// the release's asset list (`elixir-otp-<n>.zip`). Used only to make
/// the "no zip for your OTP major" error actionable. Returns `None` on
/// any fetch/parse failure (the caller degrades to a generic message).
async fn available_otp_majors(
    http: &dyn crate::effects::HttpFetcher,
    tag: &str,
) -> Option<Vec<String>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}");
    let body = http.get_text_authenticated(&url).await.ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let mut majors: Vec<u64> = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(|a| a.get("name")?.as_str())
        .filter_map(|n| n.strip_prefix("elixir-otp-")?.strip_suffix(".zip"))
        .filter_map(|m| m.parse::<u64>().ok())
        .collect();
    majors.sort_unstable();
    majors.dedup();
    Some(majors.into_iter().map(|m| m.to_string()).collect())
}

/// Version compare over up-to-4 dotted numeric components. Shared
/// shape with the erlang backend; Elixir is 3-component but Erlang
/// (used for the erlang dir scan) can be 4.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|x| {
                let n: String = x.chars().take_while(|c| c.is_ascii_digit()).collect();
                n.parse::<u64>().unwrap_or(0)
            })
            .collect()
    }
    let (mut pa, mut pb) = (parts(a), parts(b));
    let n = pa.len().max(pb.len());
    pa.resize(n, 0);
    pb.resize(n, 0);
    pa.cmp(&pb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_v_prefix() {
        assert_eq!(strip_v("v1.18.4"), "1.18.4");
        assert_eq!(strip_v("1.18.4"), "1.18.4");
    }

    #[test]
    fn otp_major_extracts_first_component() {
        assert_eq!(otp_major("28.1.2"), Some(28));
        assert_eq!(otp_major("27.3.4.3"), Some(27));
        assert_eq!(otp_major("26"), Some(26));
        assert_eq!(otp_major(""), None);
    }

    #[test]
    fn version_cmp_orders_elixir_releases() {
        let mut v = vec!["1.18.4", "1.17.3", "1.18.0", "1.16.2"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["1.18.4", "1.18.0", "1.17.3", "1.16.2"]);
    }
}
