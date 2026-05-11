//! Ruby backend — prebuilt installer.
//!
//! Downloads prebuilt Ruby from [ruby/ruby-builder](https://github.com/ruby/ruby-builder)
//! GitHub releases. No `ruby-build` dependency, no compilation. Same pattern
//! as the Python backend (python-build-standalone).
//!
//! Gem tool management (rubocop, rails, etc.) is handled inline via the
//! rubygems.org API + `gem install`, with no external dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;

use crate::backend::*;

pub struct RubyBackend;

const REPO: &str = "ruby/ruby-builder";

// ─── Platform mapping ───────────────────────────────────────────────

fn platform_suffix() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "ubuntu-22.04-x64",
        ("linux", "aarch64") => "ubuntu-22.04-arm64",
        _ => return None,
    })
}

/// qusp owns Ruby under its own paths (no separate `rv` data dir).
fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn ruby_root(p: &AnyvPaths, version: &str) -> PathBuf {
    p.data.join("ruby").join(version)
}

// ─── Tool registry ──────────────────────────────────────────────────

struct ToolEntry {
    name: &'static str,
    gem: &'static str,
    bin: &'static str,
}

const TOOL_REGISTRY: &[ToolEntry] = &[
    ToolEntry {
        name: "rubocop",
        gem: "rubocop",
        bin: "rubocop",
    },
    ToolEntry {
        name: "standard",
        gem: "standard",
        bin: "standardrb",
    },
    ToolEntry {
        name: "brakeman",
        gem: "brakeman",
        bin: "brakeman",
    },
    ToolEntry {
        name: "steep",
        gem: "steep",
        bin: "steep",
    },
    ToolEntry {
        name: "sorbet",
        gem: "sorbet",
        bin: "srb",
    },
    ToolEntry {
        name: "ruby-lsp",
        gem: "ruby-lsp",
        bin: "ruby-lsp",
    },
    ToolEntry {
        name: "solargraph",
        gem: "solargraph",
        bin: "solargraph",
    },
    ToolEntry {
        name: "bundler",
        gem: "bundler",
        bin: "bundle",
    },
    ToolEntry {
        name: "rake",
        gem: "rake",
        bin: "rake",
    },
    ToolEntry {
        name: "rspec",
        gem: "rspec",
        bin: "rspec",
    },
    ToolEntry {
        name: "rails",
        gem: "rails",
        bin: "rails",
    },
    ToolEntry {
        name: "rerun",
        gem: "rerun",
        bin: "rerun",
    },
    ToolEntry {
        name: "fasterer",
        gem: "fasterer",
        bin: "fasterer",
    },
    ToolEntry {
        name: "reek",
        gem: "reek",
        bin: "reek",
    },
    ToolEntry {
        name: "yard",
        gem: "yard",
        bin: "yard",
    },
];

fn lookup_tool(name: &str) -> Option<&'static ToolEntry> {
    TOOL_REGISTRY.iter().find(|e| e.name == name)
}

// ─── GitHub releases API ────────────────────────────────────────────

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
}

// ─── Rubygems API ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct GemInfo {
    version: String,
    #[serde(default)]
    sha: String,
}

#[derive(Deserialize)]
struct GemVersion {
    number: String,
    #[serde(default)]
    sha: String,
}

// ─── Version helpers ────────────────────────────────────────────────

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64) {
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}

/// Strip `ruby-` prefix if present (chruby/asdf-style).
fn clean_version(v: &str) -> String {
    v.trim()
        .strip_prefix("ruby-")
        .unwrap_or(v.trim())
        .to_string()
}

// ─── .ruby-version / Gemfile parsing ────────────────────────────────

fn detect_ruby_version(cwd: &Path) -> Result<Option<DetectedVersion>> {
    let mut dir: Option<&Path> = Some(cwd);
    while let Some(d) = dir {
        // Gemfile `ruby "X.Y.Z"` takes precedence
        let gemfile = d.join("Gemfile");
        if gemfile.is_file() {
            if let Some(v) = parse_gemfile_ruby(&gemfile)? {
                return Ok(Some(DetectedVersion {
                    version: v,
                    source: "gemfile".into(),
                    origin: gemfile,
                }));
            }
        }
        // .ruby-version
        let rv = d.join(".ruby-version");
        if rv.is_file() {
            let raw = std::fs::read_to_string(&rv).unwrap_or_default();
            let v = clean_version(&raw);
            if !v.is_empty() {
                return Ok(Some(DetectedVersion {
                    version: v,
                    source: ".ruby-version".into(),
                    origin: rv,
                }));
            }
        }
        dir = d.parent();
    }
    Ok(None)
}

fn parse_gemfile_ruby(gemfile: &Path) -> Result<Option<String>> {
    let content =
        std::fs::read_to_string(gemfile).with_context(|| format!("read {}", gemfile.display()))?;
    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if !line.starts_with("ruby ") && !line.starts_with("ruby\t") {
            continue;
        }
        let after = line.trim_start_matches("ruby").trim_start();
        let q = after.chars().next();
        if q != Some('"') && q != Some('\'') {
            continue;
        }
        let quote = q.unwrap();
        let rest = &after[1..];
        if let Some(end) = rest.find(quote) {
            return Ok(Some(clean_version(&rest[..end])));
        }
    }
    Ok(None)
}

// ─── Backend impl ───────────────────────────────────────────────────

#[async_trait]
impl Backend for RubyBackend {
    fn id(&self) -> &'static str {
        "ruby"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["Gemfile", ".ruby-version"]
    }
    fn knows_tool(&self, name: &str) -> bool {
        lookup_tool(name).is_some()
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        detect_ruby_version(cwd)
    }

    async fn install(
        &self,
        _qusp_paths: &AnyvPaths,
        version: &str,
        ctx: &InstallCtx<'_>,
    ) -> Result<InstallReport> {
        let http = ctx.http;
        let progress = ctx.progress;

        let paths = paths()?;
        paths.ensure_dirs()?;
        let install_dir = ruby_root(&paths, version);
        if install_dir.join("bin").join("ruby").exists() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        let _install_guard =
            crate::effects::StoreLock::acquire(&crate::effects::lock_path_for(&install_dir))?;

        let platform = platform_suffix()
            .ok_or_else(|| anyhow!("ruby-builder has no prebuilt for this platform"))?;

        let asset_name = format!("ruby-{version}-{platform}.tar.gz");
        let url =
            format!("https://github.com/{REPO}/releases/download/ruby-{version}/{asset_name}");

        let mut task = progress.start(&format!("downloading ruby {version}"), None);
        let bytes = http
            .get_bytes_streaming(&url, task.as_mut())
            .await
            .with_context(|| {
                format!(
                    "download ruby {version} from ruby-builder. \
                     Check available versions with `qusp list ruby`"
                )
            })?;
        task.finish(format!("downloaded {asset_name}"));

        // Extract into a content-addressed store slot.
        let hash_prefix = {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(&bytes);
            hex::encode(&h.finalize()[..8])
        };
        let cache_path = paths.cache.join(&asset_name);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)
            .with_context(|| format!("write {}", cache_path.display()))?;

        let store_dir = paths.store().join(&hash_prefix);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;

        // ruby-builder tarballs have a top-level dir (arm64/, x64/, etc.).
        // Find the one containing bin/ruby.
        let real_root = find_ruby_root(&store_dir)?;

        // ruby-builder binaries have hardcoded absolute paths from the
        // GitHub Actions runner. Patch them to point at the actual location.
        #[cfg(target_os = "macos")]
        patch_macos_dylib_paths(&real_root)?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        crate::effects::atomic_symlink_swap(&real_root, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                real_root.display()
            )
        })?;

        let _ = std::fs::remove_file(&cache_path);

        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        let dir = ruby_root(&paths, version);
        if !dir.exists() && !dir.is_symlink() {
            bail!("ruby {version} is not installed via qusp");
        }
        // Remove symlink (or dir) and the underlying store entry if symlinked.
        if dir.is_symlink() {
            let target = std::fs::read_link(&dir).ok();
            std::fs::remove_file(&dir)
                .with_context(|| format!("remove symlink {}", dir.display()))?;
            if let Some(t) = target {
                // Walk up to find the store slot (hash-prefixed parent).
                if let Some(store_slot) = t
                    .ancestors()
                    .find(|a| a.parent().map(|p| p.ends_with("store")).unwrap_or(false))
                {
                    std::fs::remove_dir_all(store_slot).ok();
                }
            }
        } else {
            std::fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
        }
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("ruby");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let n = e.file_name().to_string_lossy().to_string();
            if n.chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                out.push(n);
            }
        }
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn list_remote(&self, http: &dyn crate::effects::HttpFetcher) -> Result<Vec<String>> {
        let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
        let body = http.get_text_authenticated(&url).await?;
        let releases: Vec<GhRelease> =
            serde_json::from_str(&body).context("parse ruby-builder release index")?;
        let mut out: Vec<String> = releases
            .iter()
            .filter_map(|r| {
                let v = r.tag_name.strip_prefix("ruby-")?;
                // Skip previews, RCs, dev builds
                if v.contains("preview") || v.contains("rc") || v.contains("dev") {
                    return None;
                }
                Some(v.to_string())
            })
            .collect();
        out.sort_by(|a, b| version_cmp(b, a));
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        http: &dyn crate::effects::HttpFetcher,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        let client = require_reqwest(http)?;

        let gem = match spec {
            ToolSpec::Long {
                package: Some(g), ..
            } => g.clone(),
            _ => lookup_tool(name)
                .map(|e| e.gem.to_string())
                .ok_or_else(|| {
                    anyhow!(
                        "unknown tool '{name}' — pick from the registry or set `package = \"...\"` \
                         in qusp.toml"
                    )
                })?,
        };

        let raw_version = match spec {
            ToolSpec::Short(v) => v.trim().to_string(),
            ToolSpec::Long { version, .. } => version.trim().to_string(),
        };

        let (version, sha) = match raw_version.as_str() {
            "latest" | "*" => {
                let url = format!("https://rubygems.org/api/v1/gems/{gem}.json");
                let text = client
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await?;
                let info: GemInfo = serde_json::from_str(&text)?;
                (info.version, info.sha)
            }
            v => {
                let url = format!("https://rubygems.org/api/v1/versions/{gem}.json");
                let text = client
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .text()
                    .await?;
                let versions: Vec<GemVersion> = serde_json::from_str(&text)?;
                let found = versions
                    .into_iter()
                    .find(|gv| gv.number == v)
                    .ok_or_else(|| anyhow!("version {v} of {gem} not found on rubygems.org"))?;
                (found.number, found.sha)
            }
        };

        let bin = match spec {
            ToolSpec::Long { bin: Some(b), .. } => b.clone(),
            _ => lookup_tool(name)
                .map(|e| e.bin.to_string())
                .unwrap_or_else(|| name.to_string()),
        };

        Ok(ResolvedTool {
            name: name.to_string(),
            package: gem,
            version,
            bin,
            upstream_hash: sha,
        })
    }

    async fn install_tool(
        &self,
        _qusp_paths: &AnyvPaths,
        _http: &dyn crate::effects::HttpFetcher,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        let paths = paths()?;
        let ruby_dir = ruby_root(&paths, toolchain_version);
        let gem_bin = ruby_dir.join("bin").join("gem");
        if !gem_bin.exists() {
            bail!(
                "ruby {toolchain_version} not installed (looked at {})",
                gem_bin.display()
            );
        }

        let dest = tool_gem_home(
            &paths,
            toolchain_version,
            &resolved.package,
            &resolved.version,
        );
        anyv_core::paths::ensure_dir(&dest)?;

        let bin_path = dest.join("bin").join(&resolved.bin);
        if bin_path.exists() {
            return Ok(make_locked(resolved, toolchain_version));
        }

        // Prepend Ruby's bin dir to PATH so gem finds its companions.
        let bin_dir = ruby_dir.join("bin");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::from(bin_dir.as_os_str());
        new_path.push(":");
        new_path.push(&path);

        let status = Command::new(&gem_bin)
            .args([
                "install",
                &resolved.package,
                "-v",
                &resolved.version,
                "-i",
                &dest.to_string_lossy(),
                "--no-document",
                "--no-update-sources",
            ])
            .env("PATH", new_path)
            .env("GEM_HOME", &dest)
            .env("GEM_PATH", &dest)
            .status()
            .with_context(|| {
                format!(
                    "spawn gem install {}@{}",
                    resolved.package, resolved.version
                )
            })?;
        if !status.success() {
            bail!(
                "gem install {}@{} failed (exit {:?})",
                resolved.package,
                resolved.version,
                status.code()
            );
        }
        if !bin_path.exists() {
            bail!(
                "gem install produced no binary {} in {}",
                resolved.bin,
                dest.join("bin").display()
            );
        }
        Ok(make_locked(resolved, toolchain_version))
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        let paths = match paths() {
            Ok(p) => p,
            Err(_) => return PathBuf::from(&locked.bin),
        };
        tool_gem_home(&paths, &locked.built_with, &locked.package, &locked.version)
            .join("bin")
            .join(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = paths()?;
        let root = ruby_root(&paths, version);
        // ruby-builder binaries have $LOAD_PATH baked into the binary at
        // compile time. Override via RUBYLIB so stdlib + extensions resolve.
        let lib = root.join("lib");
        let ruby_ver = detect_ruby_lib_version(&lib);
        let arch = detect_ruby_arch(&lib, &ruby_ver);
        let rubylib = format!(
            "{}:{}",
            lib.join("ruby").join(&ruby_ver).display(),
            lib.join("ruby").join(&ruby_ver).join(&arch).display(),
        );
        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env: [("RUBYLIB".to_string(), rubylib)].into_iter().collect(),
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        use crate::effects::FarmBinary;
        vec![
            FarmBinary::unversioned("ruby"),
            FarmBinary::unversioned("irb"),
            FarmBinary::unversioned("gem"),
            FarmBinary::unversioned("bundle"),
            FarmBinary::unversioned("bundler"),
            FarmBinary::unversioned("rake"),
            FarmBinary::unversioned("rdoc"),
            FarmBinary::unversioned("ri"),
            FarmBinary::unversioned("erb"),
        ]
    }
}

// ─── Helpers ────────────────────────────────────────────────────────

fn require_reqwest(http: &dyn crate::effects::HttpFetcher) -> Result<&reqwest::Client> {
    http.as_reqwest_client().ok_or_else(|| {
        anyhow!(
            "ruby tool management requires a real reqwest::Client (LiveHttp); \
             the supplied HttpFetcher impl doesn't provide one"
        )
    })
}

/// After extracting a ruby-builder tarball, find the directory containing
/// `bin/ruby`. The top-level dir varies by platform (`arm64/`, `x64/`, etc.).
fn find_ruby_root(store_dir: &Path) -> Result<PathBuf> {
    for entry in
        std::fs::read_dir(store_dir).with_context(|| format!("read {}", store_dir.display()))?
    {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() && p.join("bin").join("ruby").exists() {
            return Ok(p);
        }
    }
    bail!(
        "extracted ruby-builder tarball at {} does not contain a directory with bin/ruby",
        store_dir.display()
    )
}

fn tool_gem_home(paths: &AnyvPaths, ruby_version: &str, gem: &str, gem_version: &str) -> PathBuf {
    paths
        .data
        .join("ruby-tools")
        .join(ruby_version)
        .join(gem)
        .join(gem_version)
}

/// Detect the Ruby stdlib version directory (e.g., "3.4.0") under lib/ruby/.
fn detect_ruby_lib_version(lib: &Path) -> String {
    let ruby_dir = lib.join("ruby");
    if let Ok(entries) = std::fs::read_dir(&ruby_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && e.path().is_dir()
            {
                return name;
            }
        }
    }
    "3.4.0".to_string() // fallback
}

/// Detect the platform-specific subdirectory (e.g., "x86_64-darwin24").
fn detect_ruby_arch(lib: &Path, ruby_ver: &str) -> String {
    let ver_dir = lib.join("ruby").join(ruby_ver);
    if let Ok(entries) = std::fs::read_dir(&ver_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if (name.contains("darwin") || name.contains("linux")) && e.path().is_dir() {
                return name;
            }
        }
    }
    "unknown".to_string()
}

fn make_locked(r: &ResolvedTool, ruby_version: &str) -> LockedTool {
    LockedTool {
        name: r.name.clone(),
        package: r.package.clone(),
        version: r.version.clone(),
        bin: r.bin.clone(),
        upstream_hash: r.upstream_hash.clone(),
        built_with: ruby_version.to_string(),
    }
}

// ─── macOS: fix hardcoded dylib paths from ruby-builder CI ──────────

/// ruby/ruby-builder binaries are compiled on GitHub Actions runners and
/// contain hardcoded absolute paths like
/// `/Users/runner/hostedtoolcache/Ruby/3.4.9/x64/lib/libruby.3.4.dylib`.
/// We rewrite every Mach-O reference AND text config (`rbconfig.rb`,
/// `.pc`, etc.) to point at the actual install location.
#[cfg(target_os = "macos")]
fn patch_macos_dylib_paths(ruby_root: &Path) -> Result<()> {
    let ruby_bin = ruby_root.join("bin").join("ruby");
    let output = Command::new("otool")
        .args(["-L"])
        .arg(&ruby_bin)
        .output()
        .context("otool -L bin/ruby")?;
    let otool = String::from_utf8_lossy(&output.stdout);

    // Find the old hardcoded libruby reference to derive the runner prefix.
    let old_ref = otool
        .lines()
        .filter_map(|l| {
            let s = l.trim().split_whitespace().next()?;
            if s.contains("libruby") && s.contains("/runner/") {
                Some(s.to_string())
            } else {
                None
            }
        })
        .next();

    let Some(old_ref) = old_ref else {
        return Ok(());
    };

    // Derive the runner prefix dir (everything before /lib/libruby...).
    let runner_prefix = old_ref
        .find("/lib/libruby")
        .map(|i| &old_ref[..i])
        .unwrap_or(&old_ref);
    let new_prefix = ruby_root.to_string_lossy();

    let dylib_name = Path::new(&old_ref)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let new_ref = ruby_root
        .join("lib")
        .join(&dylib_name)
        .to_string_lossy()
        .to_string();

    // 1. Fix libruby's own install name.
    let _ = Command::new("install_name_tool")
        .args(["-id", &new_ref])
        .arg(ruby_root.join("lib").join(&dylib_name))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    // 2. Walk and patch every Mach-O file that references the old path.
    patch_macho_refs_recursive(ruby_root, &old_ref, &new_ref)?;

    // 3. Rewrite text config files (rbconfig.rb, .pc, Makefiles).
    patch_text_configs_recursive(ruby_root, runner_prefix, &new_prefix)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn patch_macho_refs_recursive(dir: &Path, old_ref: &str, new_ref: &str) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            patch_macho_refs_recursive(&path, old_ref, new_ref)?;
        } else {
            let dominated = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "bundle" || e == "dylib")
                .unwrap_or(false)
                || path.parent().map(|p| p.ends_with("bin")).unwrap_or(false);
            if dominated {
                let _ = Command::new("install_name_tool")
                    .args(["-change", old_ref, new_ref])
                    .arg(&path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }
    Ok(())
}

/// Replace the runner prefix in text files that embed the install path
/// (rbconfig.rb, pkg-config .pc, extension Makefiles).
#[cfg(target_os = "macos")]
fn patch_text_configs_recursive(dir: &Path, old_prefix: &str, new_prefix: &str) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            patch_text_configs_recursive(&path, old_prefix, new_prefix)?;
        } else {
            let dominated = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e, "rb" | "pc" | "h"))
                .unwrap_or(false)
                || path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n == "Makefile")
                    .unwrap_or(false);
            if !dominated {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(old_prefix) {
                    let patched = content.replace(old_prefix, new_prefix);
                    let _ = std::fs::write(&path, patched);
                }
            }
        }
    }
    Ok(())
}
