//! Erlang/OTP backend — prebuilt distribution from `erlef/otp_builds`.
//!
//! qusp ships Erlang as a community-prebuilt OTP release tarball from
//! the Erlang Ecosystem Foundation's `erlef/otp_builds` repo. kerl /
//! source builds are deliberately NOT used: compiling OTP from source
//! is a multi-minute, dependency-heavy affair (autoconf, a C compiler,
//! optional wxWidgets / OpenSSL probing) that qusp's prebuilt-first
//! stance rules out.
//!
//! ## Two prebuilt sources, by platform
//!
//! - **macOS** (fully supported): `erlef/otp_builds` GitHub releases
//!   (Apple Silicon + Intel), verified via the Sigstore provenance
//!   digest. This is the daily-dogfood path.
//! - **Linux glibc** (experimental): `erlef/otp_builds` ships no Linux
//!   artifacts, so we fall back to the Erlang Ecosystem Foundation's
//!   `builds.hex.pm` service (the same source `setup-beam` uses),
//!   verified via the sha256 column of its `builds.txt` manifest. These
//!   are Ubuntu-built, glibc-linked tarballs — Alpine/musl is rejected.
//!   The runtime is **not yet validated in qusp's macOS-only dev loop**;
//!   it rides on Linux CI.
//! - **Windows / musl / other**: bail with a clear message.
//!
//! ## Release / asset layout
//!
//!   macOS (GitHub erlef/otp_builds):
//!     tag:    `OTP-<version>`          (e.g. `OTP-28.1.2`, `OTP-27.3.4.3`)
//!     asset:  `otp-<triple>.tar.gz` + `<asset>.sigstore`
//!
//!   Linux (builds.hex.pm):
//!     base:   `builds/otp/<arch>/<flavor>`   (arch ∈ amd64|arm64;
//!             flavor e.g. `ubuntu-22.04`)
//!     asset:  `OTP-<version>.tar.gz`
//!     verify: the 4th column of `<base>/builds.txt`
//!             (`OTP-<ver> <ref> <date> <sha256>`)
//!
//! The stored / displayed version is the tag with the `OTP-` prefix
//! stripped. OTP versions can be 2–4 dotted components.
//!
//! ## Verification (Sigstore provenance digest)
//!
//! Unlike most backends, `erlef/otp_builds` ships **no** plain
//! `SHA256.txt` / `.sha256` sidecar — only a Sigstore bundle
//! (`<asset>.sigstore`). That bundle is a DSSE-wrapped in-toto SLSA
//! provenance statement whose `subject[].digest.sha256` is the
//! tarball's sha256. We fetch the bundle, decode the DSSE payload, and
//! verify our download against that attested digest. Full cryptographic
//! verification of the Sigstore signature chain (Fulcio cert + Rekor
//! inclusion) is deferred to the v1.0 roadmap; consuming the attested
//! digest already keeps qusp's "verify a publisher-published digest
//! before extract" invariant intact.
//!
//! ## Relocation (the load-bearing step)
//!
//! The prebuilt is relocatable by design: `bin/erl` (and `bin/start`,
//! plus their `erts-*/bin` twins) resolve their own root at runtime via
//! a `find_rootdir "$0" "<build-time-fallback>"` shell helper that walks
//! up from the script's path looking for `erts-*`. That works when `erl`
//! is invoked through its real `bin/` dir — but qusp's symlink farm
//! exposes `~/.local/bin/erl` OUTSIDE the OTP root, where the walk hits
//! `/` and uses the build-time fallback (`/tmp/otp-...`), which doesn't
//! exist on the user's machine. We rewrite that fallback to the real
//! install dir so the farm symlinks resolve. (Older/source-style trees
//! ship a generated `Install` script instead; if present we run
//! `./Install -minimal <abs>` and skip the rewrite.)

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use base64::Engine;
use sha2::Digest;

use crate::backend::*;

pub struct ErlangBackend;

const REPO: &str = "erlef/otp_builds";

/// macOS prebuilt triple (erlef/otp_builds asset). `None` off macOS.
fn mac_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => return None,
    })
}

/// builds.hex.pm arch slug for the current Linux host. `None` if the
/// arch isn't published there.
fn linux_arch() -> Option<&'static str> {
    Some(match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return None,
    })
}

/// Ordered builds.hex.pm flavors to try for this host, most-preferred
/// first. builds.hex.pm coverage varies per flavor (e.g. ubuntu-24.04
/// carries no OTP-27.3), so we fall back to an older flavor when the
/// pinned version isn't built for the host's own — an older-Ubuntu build
/// runs on a newer host **within the same OpenSSL major** (22.04 and
/// 24.04 are both OpenSSL 3; 20.04 is OpenSSL 1.1 and kept on its own
/// island so a crypto NIF never chases a missing libcrypto). An explicit
/// `QUSP_OTP_UBUNTU` pins a single flavor (no fallback).
fn linux_flavor_candidates() -> Vec<String> {
    if let Ok(v) = std::env::var("QUSP_OTP_UBUNTU") {
        let v = v.trim().trim_start_matches("ubuntu-");
        if !v.is_empty() {
            return vec![format!("ubuntu-{v}")];
        }
    }
    let chain: &[&str] = match os_release_ubuntu_version().as_deref() {
        Some("20.04") => &["ubuntu-20.04"], // OpenSSL 1.1 island
        Some("22.04") => &["ubuntu-22.04"],
        Some("24.04") => &["ubuntu-24.04", "ubuntu-22.04"],
        // Newer-than-known Ubuntu, or a non-Ubuntu glibc distro: prefer
        // the broadly-built, oldest-glibc OpenSSL-3 baseline, then 24.04.
        _ => &["ubuntu-22.04", "ubuntu-24.04"],
    };
    chain.iter().map(|s| s.to_string()).collect()
}

/// The host's `VERSION_ID` if `/etc/os-release` says `ID=ubuntu`.
fn os_release_ubuntu_version() -> Option<String> {
    let txt = std::fs::read_to_string("/etc/os-release").ok()?;
    let mut id = None;
    let mut ver = None;
    for line in txt.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = Some(v.trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            ver = Some(v.trim_matches('"').to_string());
        }
    }
    if id.as_deref() == Some("ubuntu") {
        ver
    } else {
        None
    }
}

/// True on musl libc hosts (Alpine etc.), where the glibc-linked
/// builds.hex.pm tarballs won't run.
fn is_musl() -> bool {
    if Path::new("/etc/alpine-release").exists() {
        return true;
    }
    ["/lib", "/usr/lib"].iter().any(|d| {
        std::fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-"))
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn erlang_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("erlang").join(strip_otp(version))
}

/// Strip the `OTP-` tag prefix if present; otherwise return verbatim.
fn strip_otp(v: &str) -> &str {
    v.strip_prefix("OTP-").unwrap_or(v)
}

#[async_trait]
impl Backend for ErlangBackend {
    fn id(&self) -> &'static str {
        "erlang"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["rebar.config"]
    }
    fn knows_tool(&self, _: &str) -> bool {
        false
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".erlang-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: strip_otp(&v).to_string(),
                        source: ".erlang-version".into(),
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
        let v_strip = strip_otp(version).to_string();
        let install_dir = erlang_root(&paths, version);
        if install_dir.join("bin").join("erl").exists() {
            return Ok(InstallReport {
                version: v_strip,
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;

        // Per-platform: resolve the download URL + expected sha256
        // (macOS → GitHub/Sigstore, Linux glibc → builds.hex.pm manifest).
        let dl = resolve_otp_download(http, &v_strip).await?;
        let asset = dl.asset;

        let mut task = progress.start(&format!("downloading erlang {v_strip}"), None);
        let bytes = http
            .get_bytes_streaming(&dl.url, task.as_mut())
            .await
            .with_context(|| format!("download {}", dl.url))?;
        task.finish(format!("downloaded erlang {v_strip}"));

        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !dl.sha256.eq_ignore_ascii_case(&actual) {
            bail!(
                "sha256 mismatch for {asset}: expected {}, got {actual}",
                dl.sha256
            );
        }

        // Stage: write to cache, extract into the content-addressed
        // store, drop the cache copy.
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

        // Find the OTP root: the dir that directly holds `bin/erl` and an
        // `erts-*` dir. The prebuilt tarball extracts flat (root ==
        // store_dir); tolerate a nested `otp/` or single-subdir layout.
        let otp_root = find_otp_root(&store_dir).ok_or_else(|| {
            anyhow!(
                "extracted erlang archive did not contain a recognizable OTP root \
                 (bin/erl + erts-*) under {}",
                store_dir.display()
            )
        })?;

        // Relocate so launcher scripts resolve their root even when run
        // through a farm symlink. Bake the stable install-dir path (the
        // `data/erlang/<version>` symlink that we create just below).
        let mut reloc_task = progress.start(&format!("relocating erlang {v_strip}"), None);
        if let Err(e) = relocate_otp(&otp_root, &install_dir) {
            reloc_task.fail();
            return Err(e);
        }
        reloc_task.finish(format!("relocated erlang {v_strip}"));

        if !otp_root.join("bin").join("erl").is_file() {
            bail!(
                "erlang relocation completed but bin/erl missing under {}",
                otp_root.display()
            );
        }

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&otp_root, &install_dir).with_context(|| {
            format!("symlink {} → {}", install_dir.display(), otp_root.display())
        })?;

        Ok(InstallReport {
            version: v_strip,
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = erlang_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("erlang {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("erlang");
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
        // Linux pulls the version list from the builds.hex.pm manifest
        // (same source as install); macOS uses the GitHub release index.
        if std::env::consts::OS == "linux" {
            let arch = linux_arch().ok_or_else(|| {
                anyhow!("erlang Linux prebuilds (builds.hex.pm) cover x86_64 and aarch64 only")
            })?;
            // Union across the candidate flavors, since install falls
            // back across them — everything listed is actually installable.
            let mut out: Vec<String> = Vec::new();
            for flavor in linux_flavor_candidates() {
                let url = format!("https://builds.hex.pm/builds/otp/{arch}/{flavor}/builds.txt");
                let body = http.get_text(&url).await?;
                out.extend(
                    body.lines()
                        .filter_map(|l| l.split_whitespace().next())
                        .filter_map(|tag| tag.strip_prefix("OTP-"))
                        .filter(|v| !v.contains("-rc") && !v.contains("-pre"))
                        .map(|v| v.to_string()),
                );
            }
            out.sort_by(|a, b| version_cmp(b, a));
            out.dedup();
            return Ok(out);
        }

        #[derive(serde::Deserialize)]
        struct R {
            tag_name: String,
            #[serde(default)]
            prerelease: bool,
        }
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=30");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<R> =
            serde_json::from_str(&body).context("parse erlef/otp_builds release index")?;
        let mut out: Vec<String> = releases
            .into_iter()
            .filter(|r| !r.prerelease)
            .map(|r| strip_otp(&r.tag_name).to_string())
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
        bail!(
            "Erlang dependencies are managed by rebar3/mix; \
             qusp doesn't curate an Erlang tool registry."
        )
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = erlang_root(&paths, version);
        // Pin ROOTDIR explicitly so `qusp run` / the shell hook never
        // depend on `$0`-based discovery (which the farm rewrite covers
        // for the bare-command path).
        let mut env = std::collections::BTreeMap::new();
        env.insert("ERL_ROOTDIR".into(), root.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("erl"),
            FarmBinary::unversioned("erlc"),
            FarmBinary::unversioned("escript"),
            FarmBinary::unversioned("epmd"),
            FarmBinary::unversioned("dialyzer"),
            FarmBinary::unversioned("typer"),
        ]
    }
}

/// True when `dir` directly contains an `erts-*` subdirectory.
fn has_erts_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("erts-") && e.path().is_dir())
}

/// Locate the OTP root: the dir that directly holds `bin/erl` and an
/// `erts-*` dir. Tries, in order: `store_dir` itself (the flat prebuilt
/// layout), `store_dir/otp` (conventional nesting), then a one-level
/// scan of `store_dir`'s subdirectories.
fn find_otp_root(store_dir: &Path) -> Option<PathBuf> {
    // An OTP root has an `erts-*` dir and either a generated `bin/erl`
    // (macOS prebuilt, already relocated) or an `Install` script (Linux
    // source-style tree, whose bin/ is generated during relocation).
    let looks_like_otp = |d: &Path| {
        has_erts_dir(d) && (d.join("Install").is_file() || d.join("bin").join("erl").is_file())
    };
    if looks_like_otp(store_dir) {
        return Some(store_dir.to_path_buf());
    }
    let nested = store_dir.join("otp");
    if looks_like_otp(&nested) {
        return Some(nested);
    }
    for e in std::fs::read_dir(store_dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() && looks_like_otp(&p) {
            return Some(p);
        }
    }
    None
}

/// Relocate a freshly-extracted OTP tree so its launcher scripts resolve
/// `ROOTDIR` no matter where they're invoked from. `resident_root` is the
/// absolute path the install will live at (qusp's `data/erlang/<ver>`
/// symlink), which is baked into the scripts.
fn relocate_otp(otp_root: &Path, resident_root: &Path) -> Result<()> {
    // Source/legacy trees ship a generated `Install` script — run it.
    let install = otp_root.join("Install");
    if install.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&install) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o755);
                std::fs::set_permissions(&install, perms).ok();
            }
        }
        // Install validates that <ERL_ROOT> exists and is absolute, then
        // bakes it into the generated bin/ scripts as ROOTDIR. Pass the
        // real extracted dir (otp_root) — `resident_root` is the
        // `data/erlang/<ver>` symlink we only create *after* relocation,
        // so it doesn't exist yet. otp_root (the store dir) is stable and
        // is what the post-swap symlink resolves to anyway.
        let abs_root = otp_root
            .canonicalize()
            .with_context(|| format!("canonicalize {}", otp_root.display()))?;
        let abs = abs_root.to_string_lossy();
        let out = Command::new(&install)
            .current_dir(otp_root)
            .args(["-minimal", &abs])
            .output()
            .with_context(|| format!("invoke {} -minimal", install.display()))?;
        if !out.status.success() {
            bail!(
                "erlang `Install -minimal` failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        return Ok(());
    }

    // Prebuilt (erlef/otp_builds): rewrite the build-time ROOTDIR
    // fallback in every launcher script under bin/ and erts-*/bin/.
    let root_str = resident_root.to_string_lossy().to_string();
    let mut rewritten = 0usize;
    for script in launcher_scripts(otp_root) {
        // Scripts are tiny; skip large files (beam.smp, erlexec, …) so we
        // don't slurp a multi-MB binary just to fail the utf8 check.
        if std::fs::metadata(&script)
            .map(|m| m.len() > 128 * 1024)
            .unwrap_or(true)
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&script) else {
            continue; // non-utf8 binary
        };
        let (new, n) = rewrite_rootdir_fallback(&content, &root_str);
        if n > 0 {
            std::fs::write(&script, new)
                .with_context(|| format!("rewrite {}", script.display()))?;
            rewritten += n;
        }
    }
    if rewritten == 0 {
        bail!(
            "erlang prebuilt under {} had no `Install` script and no \
             ROOTDIR fallback to relocate — unrecognized layout",
            otp_root.display()
        );
    }
    Ok(())
}

/// All regular files under `otp_root/bin` and `otp_root/erts-*/bin`.
fn launcher_scripts(otp_root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![otp_root.join("bin")];
    if let Ok(rd) = std::fs::read_dir(otp_root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir()
                && p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("erts-"))
                    .unwrap_or(false)
            {
                dirs.push(p.join("bin"));
            }
        }
    }
    let mut files = Vec::new();
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    files.push(p);
                }
            }
        }
    }
    files
}

/// Rewrite the prebuilt's build-time ROOTDIR fallback to `root`. OTP
/// versions embed it two ways, both of which we handle:
///
///   ROOTDIR="/tmp/otp-<triple>"                    (e.g. OTP 27)
///   ROOTDIR=$(find_rootdir "$0" "/tmp/otp-<triple>")   (e.g. OTP 29)
///
/// We replace the double-quoted **absolute** literal that follows either
/// a `ROOTDIR="` assignment or the `find_rootdir "$0" "` 2nd arg. Quoted
/// tokens that aren't absolute literals (`"$ERL_ROOTDIR"`,
/// `"$dyn_rootdir"`, `"$ROOTDIR/erts.../bin"`) are left untouched, as are
/// unrelated absolute literals elsewhere (e.g. `LOGDIR="/var/log"`).
/// Returns `(rewritten, count)`.
fn rewrite_rootdir_fallback(content: &str, root: &str) -> (String, usize) {
    let (c1, n1) = replace_quoted_abs_after(content, "find_rootdir \"$0\" \"", root);
    let (c2, n2) = replace_quoted_abs_after(&c1, "ROOTDIR=\"", root);
    (c2, n1 + n2)
}

/// For each `needle` occurrence (which must end with an opening `"`),
/// replace the immediately-following quoted token with `root` **only when
/// it is an absolute literal** (starts with `/`). Other tokens are left
/// verbatim. Returns `(rewritten, count)`.
fn replace_quoted_abs_after(content: &str, needle: &str, root: &str) -> (String, usize) {
    let mut out = String::with_capacity(content.len() + root.len());
    let mut rest = content;
    let mut count = 0usize;
    while let Some(i) = rest.find(needle) {
        let head_end = i + needle.len();
        out.push_str(&rest[..head_end]); // up to and including the opening quote
        let after = &rest[head_end..];
        if let Some(stripped) = after.strip_prefix('/') {
            // Absolute literal — replace up to its closing quote.
            if let Some(close) = stripped.find('"') {
                out.push_str(root);
                rest = &stripped[close..]; // resume at the closing quote
                count += 1;
                continue;
            }
        }
        // Not an absolute literal (e.g. `$ERL_ROOTDIR`) — leave as-is.
        rest = after;
    }
    out.push_str(rest);
    (out, count)
}

/// A resolved OTP download: the asset filename, its URL, and the
/// publisher-published sha256 to verify against.
struct OtpDownload {
    asset: String,
    url: String,
    sha256: String,
}

/// Resolve the download URL + expected sha256 for `v_strip` on this
/// platform. macOS → erlef/otp_builds GitHub release (digest from the
/// Sigstore provenance bundle). Linux glibc → builds.hex.pm (sha256 from
/// the per-flavor `builds.txt`). Bails on Windows / musl / unsupported
/// arch.
async fn resolve_otp_download(
    http: &dyn crate::effects::HttpFetcher,
    v_strip: &str,
) -> Result<OtpDownload> {
    match std::env::consts::OS {
        "macos" => {
            let triple =
                mac_triple().ok_or_else(|| anyhow!("unsupported macOS arch for erlang"))?;
            let tag = format!("OTP-{v_strip}");
            let asset = format!("otp-{triple}.tar.gz");
            let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
            let sig_url = format!("{url}.sigstore");
            let sig_text = http
                .get_text(&sig_url)
                .await
                .with_context(|| format!("fetch {sig_url}"))?;
            let sha256 = sha256_from_sigstore_bundle(&sig_text).ok_or_else(|| {
                anyhow!("could not extract a sha256 digest from the Sigstore bundle at {sig_url}")
            })?;
            Ok(OtpDownload { asset, url, sha256 })
        }
        "linux" => {
            if is_musl() {
                bail!(
                    "erlang prebuilds (builds.hex.pm) are glibc-only — Alpine/musl is unsupported. \
                     Use a glibc distro, or build OTP with kerl/asdf."
                );
            }
            let arch = linux_arch().ok_or_else(|| {
                anyhow!("erlang Linux prebuilds (builds.hex.pm) cover x86_64 and aarch64 only")
            })?;
            // Try each candidate flavor until one publishes this version
            // with a checksum (coverage varies per flavor).
            let flavors = linux_flavor_candidates();
            for flavor in &flavors {
                let base = format!("https://builds.hex.pm/builds/otp/{arch}/{flavor}");
                let manifest = http
                    .get_text(&format!("{base}/builds.txt"))
                    .await
                    .with_context(|| {
                        format!("fetch builds.hex.pm OTP manifest ({arch}/{flavor})")
                    })?;
                if let Some(sha256) = hexpm_sha256_for(&manifest, v_strip) {
                    return Ok(OtpDownload {
                        asset: format!("OTP-{v_strip}.tar.gz"),
                        url: format!("{base}/OTP-{v_strip}.tar.gz"),
                        sha256,
                    });
                }
            }
            bail!(
                "OTP-{v_strip} not found with a checksum in builds.hex.pm {arch} \
                 (tried {}). Run `qusp list-remote erlang` for available versions, \
                 or override the Ubuntu flavor with QUSP_OTP_UBUNTU (e.g. 22.04|24.04).",
                flavors.join(", ")
            );
        }
        other => bail!("erlang via qusp supports macOS and Linux (glibc) only, not {other}"),
    }
}

/// Pick the sha256 for `OTP-<v_strip>` from a builds.hex.pm `builds.txt`
/// whose lines are `OTP-<ver> <git-ref> <date> <sha256>`. Matches the tag
/// exactly (so `27.3` doesn't match `27.3.1`) and requires the 4th
/// column to be a 64-char hex digest (older entries omit it → `None`).
fn hexpm_sha256_for(manifest: &str, v_strip: &str) -> Option<String> {
    let tag = format!("OTP-{v_strip}");
    for line in manifest.lines() {
        let mut it = line.split_whitespace();
        // Skip blank lines — the real manifest opens with one, and using
        // `?` here would abort the whole scan on it.
        let Some(first) = it.next() else { continue };
        if first != tag {
            continue;
        }
        // remaining cols: git-ref (0), date (1), sha256 (2)
        let sha = it.nth(2)?;
        return if sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(sha.to_string())
        } else {
            None
        };
    }
    None
}

/// Extract the artifact's sha256 from a Sigstore bundle. Modern bundles
/// (`bundle.v0.3`) wrap a DSSE in-toto statement whose
/// `subject[].digest.sha256` is the artifact digest; older bundles use a
/// `messageSignature.messageDigest` (base64 raw bytes). Both are handled.
fn sha256_from_sigstore_bundle(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;

    // DSSE / in-toto provenance: decode the base64 payload and read the
    // first subject digest.
    if let Some(payload_b64) = v
        .get("dsseEnvelope")
        .and_then(|e| e.get("payload"))
        .and_then(|p| p.as_str())
    {
        if let Ok(payload) = base64::engine::general_purpose::STANDARD.decode(payload_b64) {
            if let Ok(stmt) = serde_json::from_slice::<serde_json::Value>(&payload) {
                if let Some(subjects) = stmt.get("subject").and_then(|s| s.as_array()) {
                    for s in subjects {
                        if let Some(d) = s
                            .get("digest")
                            .and_then(|d| d.get("sha256"))
                            .and_then(|x| x.as_str())
                        {
                            return Some(d.to_string());
                        }
                    }
                }
            }
        }
    }

    // hashedrekord-style: messageSignature.messageDigest.digest is the
    // base64-encoded raw digest.
    if let Some(b64) = v
        .get("messageSignature")
        .and_then(|m| m.get("messageDigest"))
        .and_then(|d| d.get("digest"))
        .and_then(|x| x.as_str())
    {
        if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b64) {
            if raw.len() == 32 {
                return Some(hex::encode(raw));
            }
        }
    }

    None
}

/// Version compare over up-to-4 dotted numeric components (OTP uses
/// 2–4). Non-numeric leading runs are parsed leniently; missing
/// components are treated as 0.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> Vec<u64> {
        strip_otp(s)
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
    fn strips_otp_prefix() {
        assert_eq!(strip_otp("OTP-28.1.2"), "28.1.2");
        assert_eq!(strip_otp("27.3.4.3"), "27.3.4.3");
    }

    #[test]
    fn version_cmp_handles_variable_length() {
        // 4-component vs 3-component, and equal-prefix ordering.
        assert_eq!(
            version_cmp("27.3.4.3", "27.3.4"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(version_cmp("28.1.2", "28.1.2"), std::cmp::Ordering::Equal);
        assert_eq!(version_cmp("28.0", "27.3.4.3"), std::cmp::Ordering::Greater);

        let mut v = vec!["27.3.4", "28.1.2", "27.3.4.3", "28.0"];
        v.sort_by(|a, b| version_cmp(b, a));
        assert_eq!(v, vec!["28.1.2", "28.0", "27.3.4.3", "27.3.4"]);
    }

    #[test]
    fn mac_triple_matches_host_when_on_macos() {
        let got = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some("aarch64-apple-darwin"),
            ("macos", "x86_64") => Some("x86_64-apple-darwin"),
            _ => None,
        };
        assert_eq!(got, mac_triple());
    }

    #[test]
    fn linux_arch_slugs() {
        let got = match std::env::consts::ARCH {
            "x86_64" => Some("amd64"),
            "aarch64" => Some("arm64"),
            _ => None,
        };
        assert_eq!(got, linux_arch());
    }

    #[test]
    fn linux_flavor_override_pins_single_flavor() {
        // Bare version and `ubuntu-`-prefixed both normalize the same way,
        // and an explicit override disables the fallback chain.
        std::env::set_var("QUSP_OTP_UBUNTU", "20.04");
        assert_eq!(linux_flavor_candidates(), vec!["ubuntu-20.04"]);
        std::env::set_var("QUSP_OTP_UBUNTU", "ubuntu-24.04");
        assert_eq!(linux_flavor_candidates(), vec!["ubuntu-24.04"]);
        std::env::remove_var("QUSP_OTP_UBUNTU");
    }

    #[test]
    fn hexpm_sha256_matches_exact_tag_with_checksum() {
        // Mirrors the real manifest: a LEADING BLANK LINE (which must not
        // abort the scan), 4-col lines (with sha), a 3-col legacy line (no
        // sha), and trailing non-OTP branch rows (master/maint).
        // NB: starts with a literal newline → a leading blank line.
        let manifest = "
OTP-24.2 df48c260e74c3e9058ff8681ce9f554e6fa0fe34 2022-06-09T23:56:36Z
OTP-27.0 601a012837ea0a5c8095bf24223132824177124d 2024-05-20T09:50:35Z
OTP-27.3 05737d130706c7189a8e6750d9c2252d2cc7987e 2025-03-05T10:37:16Z e2ea265a971505cbf7d85620ab7c53b67bfac213039f4b0d75ee45bb6052dafe
OTP-27.3.1 abc 2025-04-01T00:00:00Z 1111111111111111111111111111111111111111111111111111111111111111
master bf6adb8744a589c89eb79c8ae9c49ca348f325fd 2026-05-22T12:40:33Z 9c10a88bcdade660def54ae5b366afbf3f3c1da4f3a005f84760f188090e9637
";
        assert_eq!(
            hexpm_sha256_for(manifest, "27.3").as_deref(),
            Some("e2ea265a971505cbf7d85620ab7c53b67bfac213039f4b0d75ee45bb6052dafe")
        );
        // Exact match: 27.3 must not pick up 27.3.1.
        assert_eq!(
            hexpm_sha256_for(manifest, "27.3.1").as_deref(),
            Some("1111111111111111111111111111111111111111111111111111111111111111")
        );
        // Legacy entry without a checksum → unverifiable → None.
        assert_eq!(hexpm_sha256_for(manifest, "27.0"), None);
        assert_eq!(hexpm_sha256_for(manifest, "99.9"), None);
    }

    #[test]
    fn sigstore_dsse_digest_is_extracted() {
        // Minimal DSSE bundle wrapping an in-toto SLSA statement.
        let payload = r#"{"subject":[{"name":"otp.tar.gz","digest":{"sha256":"dcf77d4a0e96c31582b27f1f77f1396d7781a1e0ac53e3840af69c5799870893"}}]}"#;
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let bundle = format!(
            r#"{{"mediaType":"application/vnd.dev.sigstore.bundle.v0.3+json","dsseEnvelope":{{"payloadType":"application/vnd.in-toto+json","payload":"{payload_b64}"}}}}"#
        );
        assert_eq!(
            sha256_from_sigstore_bundle(&bundle).as_deref(),
            Some("dcf77d4a0e96c31582b27f1f77f1396d7781a1e0ac53e3840af69c5799870893")
        );
    }

    #[test]
    fn sigstore_message_signature_digest_is_extracted() {
        // 32 zero bytes → all-zero hex.
        let raw = [0u8; 32];
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let bundle = format!(
            r#"{{"messageSignature":{{"messageDigest":{{"algorithm":"SHA2_256","digest":"{b64}"}}}}}}"#
        );
        assert_eq!(
            sha256_from_sigstore_bundle(&bundle).as_deref(),
            Some("0000000000000000000000000000000000000000000000000000000000000000")
        );
    }

    #[test]
    fn rewrites_find_rootdir_fallback_otp29_style() {
        let script = "\
#!/bin/sh
ROOTDIR=$(find_rootdir \"$0\" \"/tmp/otp-aarch64-apple-darwin\")
BINDIR=\"$ROOTDIR/erts-17.0.1/bin\"
";
        let (out, n) = rewrite_rootdir_fallback(script, "/home/u/.local/share/erlang/28.0");
        assert_eq!(n, 1);
        assert!(out.contains("find_rootdir \"$0\" \"/home/u/.local/share/erlang/28.0\""));
        assert!(!out.contains("/tmp/otp-aarch64-apple-darwin"));
        // `"$ROOTDIR/erts.../bin"` is a $-literal — left untouched.
        assert!(out.contains("BINDIR=\"$ROOTDIR/erts-17.0.1/bin\""));
    }

    #[test]
    fn rewrites_rootdir_fallback_otp27_style_and_preserves_var_and_logdir() {
        // OTP 27 shape: literal ROOTDIR= plus $-var ROOTDIR= lines, and an
        // unrelated absolute literal (LOGDIR) that must NOT be rewritten.
        let script = "\
#!/bin/sh
LOGDIR=\"/var/log\"
if [ -z \"$ERL_ROOTDIR\" ]
then
    ROOTDIR=\"/tmp/otp-aarch64-apple-darwin\"
    if [ \"$dyn_rootdir\" != \"$ROOTDIR\" ] && [ \"$dyn_rootdir\" != \"\" ]
    then
        ROOTDIR=\"$dyn_rootdir\"
    fi
else
    ROOTDIR=\"$ERL_ROOTDIR\"
fi
";
        let (out, n) = rewrite_rootdir_fallback(script, "/data/erlang/27.3.4.3");
        assert_eq!(n, 1, "exactly the build-time literal is rewritten");
        assert!(out.contains("ROOTDIR=\"/data/erlang/27.3.4.3\""));
        assert!(!out.contains("/tmp/otp-aarch64-apple-darwin"));
        // $-var assignments and the unrelated LOGDIR literal are intact.
        assert!(out.contains("ROOTDIR=\"$ERL_ROOTDIR\""));
        assert!(out.contains("ROOTDIR=\"$dyn_rootdir\""));
        assert!(out.contains("LOGDIR=\"/var/log\""));
    }

    #[test]
    fn rewrite_reports_zero_when_no_fallback() {
        let (_, n) = rewrite_rootdir_fallback("#!/bin/sh\nexec beam\n", "/x");
        assert_eq!(n, 0);
    }
}
