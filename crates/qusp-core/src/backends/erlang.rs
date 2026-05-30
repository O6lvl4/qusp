//! Erlang/OTP backend — prebuilt distribution from `erlef/otp_builds`.
//!
//! qusp ships Erlang as a community-prebuilt OTP release tarball from
//! the Erlang Ecosystem Foundation's `erlef/otp_builds` repo. kerl /
//! source builds are deliberately NOT used: compiling OTP from source
//! is a multi-minute, dependency-heavy affair (autoconf, a C compiler,
//! optional wxWidgets / OpenSSL probing) that qusp's prebuilt-first
//! stance rules out.
//!
//! ## macOS-only (for now)
//!
//! `erlef/otp_builds` only publishes macOS prebuilts (Apple Silicon +
//! Intel) — there are no Linux/Windows artifacts in its releases. On
//! those platforms we bail with a clear message rather than silently
//! falling back to a source build.
//!
//! ## Release / asset layout
//!
//!   tag:    `OTP-<version>`            (e.g. `OTP-28.1.2`, `OTP-27.3.4.3`)
//!   asset:  `otp-aarch64-apple-darwin.tar.gz`
//!           `otp-x86_64-apple-darwin.tar.gz`
//!           `<asset>.sigstore`        (one Sigstore bundle per tarball)
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

/// macOS-only prebuilt triple. Linux/Windows return `None` and the
/// caller bails with the TODO message.
fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => return None,
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

        let triple = target_triple().ok_or_else(|| {
            anyhow!(
                "erlang prebuilds via qusp are macOS-only — erlef/otp_builds \
                 publishes no Linux/Windows artifacts"
            )
        })?;

        let tag = format!("OTP-{v_strip}");
        let asset = format!("otp-{triple}.tar.gz");
        let asset_url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
        let sig_url = format!("{asset_url}.sigstore");

        // Verify against the digest attested in the Sigstore provenance
        // bundle (there is no plain sha256 sidecar upstream).
        let sig_text = http
            .get_text(&sig_url)
            .await
            .with_context(|| format!("fetch {sig_url}"))?;
        let expected = sha256_from_sigstore_bundle(&sig_text).ok_or_else(|| {
            anyhow!("could not extract a sha256 digest from the Sigstore bundle at {sig_url}")
        })?;

        let mut task = progress.start(&format!("downloading erlang {v_strip}"), None);
        let bytes = http
            .get_bytes_streaming(&asset_url, task.as_mut())
            .await
            .with_context(|| format!("download {asset_url}"))?;
        task.finish(format!("downloaded erlang {v_strip}"));

        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!("sha256 mismatch for {asset}: expected {expected}, got {actual}");
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
    let looks_like_otp = |d: &Path| d.join("bin").join("erl").is_file() && has_erts_dir(d);
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
        let abs = resident_root.to_string_lossy();
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
    fn triple_is_macos_only() {
        let got = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Some("aarch64-apple-darwin"),
            ("macos", "x86_64") => Some("x86_64-apple-darwin"),
            _ => None,
        };
        assert_eq!(got, target_triple());
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
