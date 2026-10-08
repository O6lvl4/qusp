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

use super::common;

pub struct RubyBackend;

const REPO: &str = "ruby/ruby-builder";

// ─── Platform mapping ───────────────────────────────────────────────

fn platform_suffix() -> Option<String> {
    Some(match common::os_arch() {
        ("macos", "aarch64") => "darwin-arm64".to_string(),
        ("macos", "x86_64") => "darwin-x64".to_string(),
        ("linux", arch) => {
            let ubuntu_ver = detect_ubuntu_version().unwrap_or("22.04".to_string());
            let arch_label = match arch {
                "aarch64" => "arm64",
                _ => "x64",
            };
            format!("ubuntu-{ubuntu_ver}-{arch_label}")
        }
        _ => return None,
    })
}

/// Detect the Ubuntu version from /etc/os-release and map it to the
/// nearest ruby-builder build target (currently 22.04 or 24.04).
fn detect_ubuntu_version() -> Option<String> {
    let content = std::fs::read_to_string("/etc/os-release").ok()?;
    if !content.lines().any(|l| l.starts_with("ID=ubuntu")) {
        return None;
    }
    let raw = content
        .lines()
        .find_map(|l| l.strip_prefix("VERSION_ID="))?;
    let ver = raw.trim_matches('"');
    // ruby-builder publishes for 22.04 and 24.04. Map detected versions
    // to the closest compatible target.
    let major_minor: f64 = ver.parse().unwrap_or(22.04);
    if major_minor >= 24.04 {
        Some("24.04".to_string())
    } else {
        Some("22.04".to_string())
    }
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

        let paths = common::qusp_paths()?;
        paths.ensure_dirs()?;
        let install_dir = common::lang_root(&paths, "ruby", version);
        if let Some(report) = common::check_already_installed(&install_dir, "bin/ruby", version) {
            return Ok(report);
        }

        let _install_guard = common::acquire_install_lock(&install_dir)?;

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
        {
            patch_macos_dylib_paths(&real_root)?;
            // Vendor the Homebrew libs the extensions link against (gmp,
            // openssl@3, libyaml, …) so ruby works without Homebrew.
            vendor_homebrew_dylibs(&real_root, http).await?;
            // ruby-builder binaries aren't relocatable — expose them through
            // env-setting farm wrappers instead of bare symlinks.
            write_farm_wrappers(&real_root)?;
        }

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
        common::uninstall_version("ruby", version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        common::list_installed_versions("ruby")
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
        out.sort_by(|a, b| common::version_cmp(b, a));
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
        let paths = common::qusp_paths()?;
        let ruby_dir = common::lang_root(&paths, "ruby", toolchain_version);
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

        let mut cmd = Command::new(&gem_bin);
        cmd.args([
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
        .env("GEM_HOME", &dest);

        // The relocated ruby needs its full env even to run `gem` itself —
        // rubygems loads stdlib and fetches over TLS via the vendored
        // openssl. GEM_PATH also gets the store's default gems.
        #[cfg(target_os = "macos")]
        {
            let store_gems = {
                let lib = ruby_dir.join("lib");
                let ver = detect_ruby_lib_version(&lib);
                lib.join("ruby").join("gems").join(&ver)
            };
            cmd.env(
                "GEM_PATH",
                format!("{}:{}", dest.display(), store_gems.display()),
            );
            for (k, v) in ruby_env_vars(&ruby_dir) {
                if k == "RUBYLIB" || k == "DYLD_FALLBACK_LIBRARY_PATH" {
                    cmd.env(k, v);
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        cmd.env("GEM_PATH", &dest);

        let status = cmd.status().with_context(|| {
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
        let paths = match common::qusp_paths() {
            Ok(p) => p,
            Err(_) => return PathBuf::from(&locked.bin),
        };
        tool_gem_home(&paths, &locked.built_with, &locked.package, &locked.version)
            .join("bin")
            .join(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = common::qusp_paths()?;
        let root = common::lang_root(&paths, "ruby", version);
        let mut env: std::collections::BTreeMap<String, String> = Default::default();

        // ruby-builder binaries bake $LOAD_PATH (and, on macOS, Homebrew
        // dylib paths) at compile time. Override via env so stdlib, RubyGems,
        // and the vendored Homebrew libs resolve at the real install location.
        #[cfg(target_os = "macos")]
        for (k, v) in ruby_env_vars(&root) {
            env.insert(k, v);
        }

        #[cfg(target_os = "linux")]
        {
            let lib = root.join("lib");
            let ruby_ver = detect_ruby_lib_version(&lib);
            let arch = detect_ruby_arch(&lib, &ruby_ver);
            env.insert(
                "RUBYLIB".to_string(),
                format!(
                    "{}:{}",
                    lib.join("ruby").join(&ruby_ver).display(),
                    lib.join("ruby").join(&ruby_ver).join(&arch).display(),
                ),
            );
            // libruby.so lives inside the install tree; loader needs it.
            env.insert("LD_LIBRARY_PATH".to_string(), lib.display().to_string());
        }

        Ok(RunEnv {
            path_prepend: vec![root.join("bin")],
            env,
        })
    }

    fn farm_binaries(&self, _version: &str) -> Vec<crate::effects::FarmBinary> {
        // On macOS the farm entries point at env-setting wrapper scripts
        // (`farm/<name>`) rather than the non-relocatable real binaries.
        #[cfg(target_os = "macos")]
        {
            use crate::effects::{FarmBinary, FarmKind};
            RUBY_FARM_BINS
                .iter()
                .map(|b| FarmBinary {
                    source: format!("farm/{b}"),
                    link_name: (*b).to_string(),
                    kind: FarmKind::Unversioned,
                })
                .collect()
        }
        #[cfg(not(target_os = "macos"))]
        {
            use crate::effects::FarmBinary;
            RUBY_FARM_BINS
                .iter()
                .map(|b| FarmBinary::unversioned(*b))
                .collect()
        }
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
            let s = l.split_whitespace().next()?;
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
                    .unwrap_or(false)
                // Extension-less scripts under bin/ (gem, bundle, rake, …) bake
                // the runner path into their `#!` shebang line — rewrite those too.
                || path.parent().map(|p| p.ends_with("bin")).unwrap_or(false);
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

// ─── macOS: vendor the Homebrew dylib closure ───────────────────────
//
// ruby-builder's macOS binaries are compiled on GitHub Actions runners
// where Homebrew is present, so `bin/ruby`, `libruby`, and native
// extension `.bundle`s link against Homebrew libs by absolute path
// (`/opt/homebrew/opt/<formula>/lib/<dylib>` — gmp, openssl@3, libyaml).
// On a machine without Homebrew those fail to load. We download the full
// closure of referenced Homebrew formulae (as bottles, straight from
// ghcr) into `<root>/vendor-lib/` and expose it at runtime via
// `DYLD_FALLBACK_LIBRARY_PATH` (set in build_run_env, the farm wrappers,
// and gem-install) — keeping qusp free of any Homebrew requirement.

#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct BrewFormula {
    bottle: BrewBottle,
}
#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct BrewBottle {
    stable: BrewBottleStable,
}
#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct BrewBottleStable {
    files: std::collections::HashMap<String, BrewBottleFile>,
}
#[cfg(target_os = "macos")]
#[derive(serde::Deserialize)]
struct BrewBottleFile {
    url: String,
    sha256: String,
}

/// Vendor every Homebrew dylib the ruby tree links against into
/// `<root>/vendor-lib/`. Best-effort: a formula that can't be fetched is
/// logged and skipped rather than failing the whole install.
#[cfg(target_os = "macos")]
async fn vendor_homebrew_dylibs(root: &Path, http: &dyn crate::effects::HttpFetcher) -> Result<()> {
    let mut queue: Vec<String> = collect_homebrew_formulae(root).into_iter().collect();
    if queue.is_empty() {
        return Ok(());
    }
    let vendor = root.join("vendor-lib");
    anyv_core::paths::ensure_dir(&vendor)?;
    let client = require_reqwest(http)?;
    let tag = macos_bottle_tag();

    let mut seen: std::collections::BTreeSet<String> = Default::default();
    while let Some(formula) = queue.pop() {
        if !seen.insert(formula.clone()) {
            continue;
        }
        match fetch_and_extract_bottle(client, &formula, &tag, &vendor).await {
            Ok(new_dylibs) => {
                // Follow the closure: a vendored dylib may itself link more
                // Homebrew formulae (e.g. openssl@3 → nothing, but be safe).
                for d in &new_dylibs {
                    for f in collect_homebrew_formulae_in_file(d) {
                        if !seen.contains(&f) {
                            queue.push(f);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("qusp: could not vendor Homebrew formula '{formula}': {e:#}"),
        }
    }

    // arm64 refuses to load unsigned code; ad-hoc re-sign each vendored lib.
    if let Ok(rd) = std::fs::read_dir(&vendor) {
        for e in rd.flatten() {
            let _ = Command::new("codesign")
                .args(["-f", "-s", "-"])
                .arg(e.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
    Ok(())
}

/// Homebrew formula names referenced anywhere in the tree's Mach-O files.
#[cfg(target_os = "macos")]
fn collect_homebrew_formulae(root: &Path) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    collect_hb_recursive(root, &mut out);
    out
}

#[cfg(target_os = "macos")]
fn collect_hb_recursive(dir: &Path, out: &mut std::collections::BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_hb_recursive(&p, out);
            continue;
        }
        let macho = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x == "dylib" || x == "bundle")
            .unwrap_or(false)
            || p.parent().map(|d| d.ends_with("bin")).unwrap_or(false);
        if macho {
            for f in collect_homebrew_formulae_in_file(&p) {
                out.insert(f);
            }
        }
    }
}

/// Parse `otool -L` output for `/opt/homebrew/opt/<formula>/…` (and the
/// Intel `/usr/local/opt/…`) references, returning the formula names.
#[cfg(target_os = "macos")]
fn collect_homebrew_formulae_in_file(file: &Path) -> Vec<String> {
    let Ok(o) = Command::new("otool").args(["-L"]).arg(file).output() else {
        return vec![];
    };
    let text = String::from_utf8_lossy(&o.stdout);
    let mut v = vec![];
    for line in text.lines() {
        let s = line.split_whitespace().next().unwrap_or("");
        for prefix in ["/opt/homebrew/opt/", "/usr/local/opt/"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                if let Some(f) = rest.split('/').next() {
                    if !f.is_empty() {
                        v.push(f.to_string());
                    }
                }
            }
        }
    }
    v
}

/// Fetch one Homebrew formula's bottle and copy its `*.dylib`s into
/// `vendor`. Returns the paths of dylibs newly copied.
#[cfg(target_os = "macos")]
async fn fetch_and_extract_bottle(
    client: &reqwest::Client,
    formula: &str,
    tag: &str,
    vendor: &Path,
) -> Result<Vec<PathBuf>> {
    let api = format!("https://formulae.brew.sh/api/formula/{formula}.json");
    let text = client
        .get(&api)
        .header("User-Agent", "qusp")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let f: BrewFormula =
        serde_json::from_str(&text).with_context(|| format!("parse formula json for {formula}"))?;
    let file = pick_bottle(&f.bottle.stable.files, tag)
        .ok_or_else(|| anyhow!("no bottle for {formula} (looked for tag {tag})"))?;

    // ghcr blob download uses the well-known anonymous bearer token.
    let bytes = client
        .get(&file.url)
        .header("Authorization", "Bearer QQ==")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(&bytes);
        let got = hex::encode(h.finalize());
        if got != file.sha256 {
            bail!(
                "sha256 mismatch for {formula} bottle (got {got}, want {})",
                file.sha256
            );
        }
    }

    let tmp = vendor.join(format!(".extract-{}", formula.replace(['/', '@'], "_")));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).ok();
    }
    anyv_core::paths::ensure_dir(&tmp)?;
    let tarball = tmp.join("bottle.tar.gz");
    std::fs::write(&tarball, &bytes)?;
    extract_archive(&tarball, &tmp)?;

    let mut copied = vec![];
    copy_dylibs_recursive(&tmp, vendor, &mut copied)?;
    std::fs::remove_dir_all(&tmp).ok();
    Ok(copied)
}

/// Choose the bottle file for this platform: exact OS tag, then older
/// macOS fallbacks (bottles are forward-compatible), then `all`.
#[cfg(target_os = "macos")]
fn pick_bottle<'a>(
    files: &'a std::collections::HashMap<String, BrewBottleFile>,
    tag: &str,
) -> Option<&'a BrewBottleFile> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64_"
    } else {
        ""
    };
    let candidates = [
        tag.to_string(),
        format!("{arch}tahoe"),
        format!("{arch}sequoia"),
        format!("{arch}sonoma"),
        format!("{arch}ventura"),
        format!("{arch}monterey"),
        "all".to_string(),
    ];
    candidates.iter().find_map(|k| files.get(k))
}

#[cfg(target_os = "macos")]
fn copy_dylibs_recursive(dir: &Path, vendor: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dylibs_recursive(&p, vendor, out)?;
            continue;
        }
        if p.extension().and_then(|x| x.to_str()) == Some("dylib") {
            if let Some(name) = p.file_name() {
                let dest = vendor.join(name);
                if !dest.exists() {
                    std::fs::copy(&p, &dest).with_context(|| format!("copy {}", p.display()))?;
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
                    out.push(dest);
                }
            }
        }
    }
    Ok(())
}

/// Map the running macOS version to its Homebrew bottle tag.
#[cfg(target_os = "macos")]
fn macos_bottle_tag() -> String {
    let major = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().split('.').next().map(str::to_string))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(15);
    let name = match major {
        26.. => "tahoe",
        15 => "sequoia",
        14 => "sonoma",
        13 => "ventura",
        _ => "monterey",
    };
    if cfg!(target_arch = "aarch64") {
        format!("arm64_{name}")
    } else {
        name.to_string()
    }
}

// ─── macOS: runtime env + farm wrappers ─────────────────────────────

/// The env that makes a relocated ruby-builder ruby fully functional:
/// stdlib/RubyGems load paths, gem paths, and the vendored Homebrew libs.
#[cfg(target_os = "macos")]
fn ruby_env_vars(root: &Path) -> Vec<(String, String)> {
    let lib = root.join("lib");
    let ver = detect_ruby_lib_version(&lib);
    let arch = detect_ruby_arch(&lib, &ver);
    let ruby_lib = lib.join("ruby");
    let rubylib = format!(
        "{}:{}:{}",
        ruby_lib.join(&ver).display(),
        ruby_lib.join(&ver).join(&arch).display(),
        ruby_lib.join("site_ruby").join(&ver).display(),
    );
    let gems = ruby_lib.join("gems").join(&ver);
    let vendor = root.join("vendor-lib");
    vec![
        ("RUBYLIB".to_string(), rubylib),
        ("GEM_HOME".to_string(), gems.display().to_string()),
        ("GEM_PATH".to_string(), gems.display().to_string()),
        (
            "DYLD_FALLBACK_LIBRARY_PATH".to_string(),
            format!("{}:/usr/lib", vendor.display()),
        ),
    ]
}

/// Write farm wrapper scripts under `<root>/farm/`. ruby-builder binaries
/// aren't relocatable (their `$LOAD_PATH` is baked to the CI runner path),
/// so a bare farm symlink can't find stdlib/gems/dylibs. Each wrapper sets
/// the env from [`ruby_env_vars`] and execs the real binary — the farm
/// symlinks point here instead of at `bin/`.
#[cfg(target_os = "macos")]
fn write_farm_wrappers(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let farm = root.join("farm");
    anyv_core::paths::ensure_dir(&farm)?;
    let exports: String = ruby_env_vars(root)
        .iter()
        .map(|(k, v)| format!("export {k}=\"{v}\"\n"))
        .collect();
    for b in RUBY_FARM_BINS {
        let real = root.join("bin").join(b);
        if !real.exists() {
            continue;
        }
        let script = format!(
            "#!/bin/sh\n# generated by qusp — relocated ruby-builder wrapper\n{exports}exec \"{}\" \"$@\"\n",
            real.display()
        );
        let wp = farm.join(b);
        std::fs::write(&wp, script).with_context(|| format!("write wrapper {}", wp.display()))?;
        std::fs::set_permissions(&wp, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Ruby binaries exposed in the global farm (as wrappers on macOS).
const RUBY_FARM_BINS: &[&str] = &[
    "ruby", "irb", "gem", "bundle", "bundler", "rake", "rdoc", "ri", "erb",
];
