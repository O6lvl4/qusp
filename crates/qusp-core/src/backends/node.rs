//! Node backend — native installer.
//!
//! Toolchain: nodejs.org official tarballs (`.tar.gz`) verified against
//! the per-release `SHASUMS256.txt`. Mirrors the python-build-standalone
//! pattern: download → sha256 verify → extract → content-addressed store
//! → version-named symlink.
//!
//! Tools: a small curated registry of canonical npm CLIs (`pnpm`, `yarn`,
//! `tsx`, `typescript`, `prettier`, `eslint`, `vite`, `turbo`,
//! `npm-check-updates`, `rimraf`). Tool installs hit the npm registry
//! directly: download `dist.tarball`, verify against `dist.integrity`
//! (sha512 base64), extract, mark the bin script executable. The bin
//! script's `#!/usr/bin/env node` shebang resolves to qusp's node when
//! qusp's bin/ is in PATH (`qusp run` injects exactly that).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct NodeBackend;

const DIST_BASE: &str = "https://nodejs.org/dist";
const NPM_REGISTRY: &str = "https://registry.npmjs.org";

/// Curated tool registry. The key is the **bin name** users will type
/// at the prompt (so `qusp add tool tsc` locks under the same key
/// `qusp run tsc` looks up). Restricted to npm CLIs that ship as
/// **self-contained bundles** — straight tarball extract works without
/// resolving peer-deps. For anything more complex (eslint, tsx, vite),
/// users pin manually under `[node.tools]` or use `npx`.
const REGISTRY: &[(&str, &str)] = &[
    // (qusp tool name = bin name, npm package name)
    ("pnpm", "pnpm"),
    ("yarn", "yarn"),
    ("tsc", "typescript"),
    ("prettier", "prettier"),
];

pub fn registry_lookup(name: &str) -> Option<&'static str> {
    REGISTRY.iter().find(|(k, _)| *k == name).map(|(_, p)| *p)
}

fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => return None,
    })
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn node_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("node").join(strip_v(version))
}

fn tools_root(p: &AnyvPaths) -> PathBuf {
    p.data.join("node-tools")
}

fn strip_v(v: &str) -> &str {
    v.strip_prefix('v').unwrap_or(v)
}

fn with_v(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

#[derive(Deserialize, Debug)]
struct NodeIndexEntry {
    version: String,
    #[serde(default)]
    lts: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct NpmPackument {
    version: String,
    dist: NpmDist,
    #[serde(default)]
    bin: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct NpmDist {
    tarball: String,
    integrity: String,
}

#[async_trait]
impl Backend for NodeBackend {
    fn id(&self) -> &'static str {
        "node"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[".nvmrc", ".node-version", "package.json"]
    }
    fn knows_tool(&self, name: &str) -> bool {
        registry_lookup(name).is_some()
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            for fname in [".nvmrc", ".node-version"] {
                let f = d.join(fname);
                if f.is_file() {
                    let raw = std::fs::read_to_string(&f).unwrap_or_default();
                    let v = raw.trim().to_string();
                    if !v.is_empty() && !v.starts_with("lts/") {
                        return Ok(Some(DetectedVersion {
                            version: strip_v(&v).to_string(),
                            source: fname.into(),
                            origin: f,
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
        _qusp_paths: &AnyvPaths,
        version: &str,
        _opts: &InstallOpts,
        http: &dyn crate::effects::HttpFetcher,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = node_root(&paths, version);
        if install_dir.join("bin").join("node").exists() {
            return Ok(InstallReport {
                version: strip_v(version).to_string(),
                install_dir,
                already_present: true,
            });
        }
        let triple =
            target_triple().ok_or_else(|| anyhow!("nodejs.org has no asset for this platform"))?;
        let v = with_v(version);
        let asset = format!("node-{v}-{triple}.tar.gz");
        let asset_url = format!("{DIST_BASE}/{v}/{asset}");
        let sums_url = format!("{DIST_BASE}/{v}/SHASUMS256.txt");

        let sums = http
            .get_text(&sums_url)
            .await
            .with_context(|| format!("fetch {sums_url}"))?;
        let expected = parse_shasums_line(&sums, &asset)
            .ok_or_else(|| anyhow!("no entry for {asset} in SHASUMS256.txt"))?;

        let bytes = http
            .get_bytes(&asset_url)
            .await
            .with_context(|| format!("download {asset_url}"))?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if expected != actual {
            bail!("sha256 mismatch for {asset}: expected {expected}, got {actual}");
        }

        let cache_path = paths.cache.join(&asset);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)
            .with_context(|| format!("write {}", cache_path.display()))?;
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;

        // The tarball expands to `node-<v>-<triple>/{bin,include,lib,share}`.
        let inner = store_dir.join(format!("node-{v}-{triple}"));
        let real_install = if inner.is_dir() {
            inner
        } else {
            store_dir.clone()
        };

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        if install_dir.exists() || install_dir.is_symlink() {
            let _ = std::fs::remove_file(&install_dir);
            let _ = std::fs::remove_dir_all(&install_dir);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_install, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_install.display()
            )
        })?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real_install, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_install.display()
            )
        })?;
        let _ = std::fs::remove_file(&cache_path);
        Ok(InstallReport {
            version: strip_v(version).to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = node_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("node {version} is not installed via qusp");
        }
        std::fs::remove_file(&dir)
            .or_else(|_| std::fs::remove_dir_all(&dir))
            .with_context(|| format!("remove {}", dir.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("node");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let url = format!("{DIST_BASE}/index.json");
        let body = http.get_text(&url).await?;
        let entries: Vec<NodeIndexEntry> =
            serde_json::from_str(&body).context("parse nodejs.org index.json")?;
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            // Surface LTS names alongside the version (informational).
            let is_lts = !matches!(e.lts, serde_json::Value::Bool(false));
            let suffix = if is_lts { " (LTS)" } else { "" };
            out.push(format!("{}{suffix}", strip_v(&e.version)));
        }
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        http: &dyn crate::effects::HttpFetcher,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        let pkg = spec
            .package_override()
            .map(String::from)
            .or_else(|| registry_lookup(name).map(String::from))
            .ok_or_else(|| {
                anyhow!(
                    "node tool '{name}' is not in the curated registry. \
                     Pin it explicitly: [node.tools] {name} = {{ package = \"<npm-name>\", version = \"latest\" }}"
                )
            })?;
        let version = spec.version();
        let url = if version == "latest" {
            format!("{NPM_REGISTRY}/{pkg}/latest")
        } else {
            format!("{NPM_REGISTRY}/{pkg}/{version}")
        };
        let body = http
            .get_text(&url)
            .await
            .with_context(|| format!("fetch {url}"))?;
        let p: NpmPackument = serde_json::from_str(&body)
            .with_context(|| format!("parse npm packument for {pkg}"))?;
        let bin = pick_bin(&p.bin, name, &pkg).unwrap_or_else(|| name.to_string());
        Ok(ResolvedTool {
            name: name.to_string(),
            package: pkg,
            version: p.version,
            bin,
            upstream_hash: p.dist.integrity,
        })
    }

    async fn install_tool(
        &self,
        _qusp_paths: &AnyvPaths,
        http: &dyn crate::effects::HttpFetcher,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        let paths = paths()?;
        // Refetch the packument to get the canonical tarball URL.
        let url = format!("{}/{}/{}", NPM_REGISTRY, resolved.package, resolved.version);
        let body = http
            .get_text(&url)
            .await
            .with_context(|| format!("fetch {url}"))?;
        let p: NpmPackument = serde_json::from_str(&body).context("parse npm packument")?;
        let tarball = p.dist.tarball;
        let integrity = p.dist.integrity.clone();
        let bytes = http
            .get_bytes(&tarball)
            .await
            .with_context(|| format!("download {tarball}"))?;
        verify_npm_integrity(&integrity, &bytes)
            .with_context(|| format!("integrity check failed for {}", resolved.package))?;

        // Content-addressed store: shorten sha-prefix derived from the
        // integrity hash for collision-free install dirs.
        let prefix = integrity_hex_prefix(&integrity).unwrap_or_else(|| "unknown".into());
        let store_dir = tools_root(&paths)
            .join(&resolved.package)
            .join(&resolved.version)
            .join(&prefix);
        if store_dir.join("package").is_dir() {
            // Already present — happy path.
        } else {
            anyv_core::paths::ensure_dir(&store_dir)?;
            let cache_path = paths
                .cache
                .join(format!("{}-{}.tgz", resolved.package, resolved.version));
            anyv_core::paths::ensure_dir(&paths.cache)?;
            std::fs::write(&cache_path, &bytes)?;
            extract_archive(&cache_path, &store_dir)?;
            let _ = std::fs::remove_file(&cache_path);
        }

        // npm tarballs always extract to a top-level `package/` dir.
        let pkg_dir = store_dir.join("package");
        let bin_path = resolve_bin(&pkg_dir, &p.bin, &resolved.name, &resolved.package)?;
        // Some tarballs ship bin scripts without the executable bit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&bin_path) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o111);
                let _ = std::fs::set_permissions(&bin_path, perms);
            }
        }
        Ok(LockedTool {
            name: resolved.name.clone(),
            package: resolved.package.clone(),
            version: resolved.version.clone(),
            bin: bin_path.to_string_lossy().into_owned(),
            upstream_hash: integrity,
            built_with: strip_v(toolchain_version).to_string(),
        })
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = node_root(&paths, version);
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: Default::default(),
        })
    }
}

fn pick_bin(bin: &serde_json::Value, tool_name: &str, pkg: &str) -> Option<String> {
    match bin {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            // Prefer the qusp tool name, fall back to package name, then any.
            map.get(tool_name)
                .or_else(|| map.get(pkg))
                .or_else(|| map.values().next())
                .and_then(|v| v.as_str())
                .map(String::from)
        }
        _ => None,
    }
}

fn resolve_bin(
    pkg_dir: &Path,
    bin: &serde_json::Value,
    tool_name: &str,
    pkg: &str,
) -> Result<PathBuf> {
    let rel = pick_bin(bin, tool_name, pkg).ok_or_else(|| {
        anyhow!(
            "package {pkg} has no `bin` field; cannot determine entrypoint for tool '{tool_name}'"
        )
    })?;
    let p = pkg_dir.join(&rel);
    if !p.is_file() {
        bail!(
            "bin script {} (declared in package.json) is missing inside the tarball",
            p.display()
        );
    }
    Ok(p)
}

/// Pure: parse one matching line out of a `SHASUMS256.txt` body.
/// Format is `<hash>  <filename>`.
pub(crate) fn parse_shasums_line(body: &str, asset: &str) -> Option<String> {
    body.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let hash = parts.next()?;
        let filename = parts.next()?;
        if filename == asset {
            Some(hash.to_string())
        } else {
            None
        }
    })
}

/// Verify an npm `dist.integrity` value (`sha512-<base64>` or `sha256-…`).
/// npm's spec allows space-separated alternatives but in practice every
/// publisher emits one; if the first verifies, we accept.
fn verify_npm_integrity(integrity: &str, bytes: &[u8]) -> Result<()> {
    let spec = integrity
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty integrity field"))?;
    let (algo, b64) = spec
        .split_once('-')
        .ok_or_else(|| anyhow!("malformed integrity '{spec}'"))?;
    let expected = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .with_context(|| format!("decode integrity for {algo}"))?;
    let actual: Vec<u8> = match algo {
        "sha512" => sha2::Sha512::digest(bytes).to_vec(),
        "sha384" => {
            use sha2::Sha384;
            Sha384::digest(bytes).to_vec()
        }
        "sha256" => sha2::Sha256::digest(bytes).to_vec(),
        other => bail!("integrity algorithm '{other}' is not supported"),
    };
    if actual == expected {
        Ok(())
    } else {
        bail!(
            "{algo} integrity mismatch for npm tarball ({} bytes expected, {} bytes computed)",
            expected.len(),
            actual.len()
        )
    }
}

/// Take the integrity hash bytes (post-base64-decode) and hex-prefix
/// the first 8 bytes for use as a content-addressed store dir.
fn integrity_hex_prefix(integrity: &str) -> Option<String> {
    let (_, b64) = integrity.split_whitespace().next()?.split_once('-')?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some(hex::encode(&bytes[..8.min(bytes.len())]))
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let s = s.strip_prefix('v').unwrap_or(s);
        // strip "(LTS)" suffix from list_remote output if it's used as a key
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
