//! The per-language Backend trait and its supporting value types.
//!
//! Every language qusp supports implements this trait. The CLI never talks
//! to a specific backend directly; it asks the [`BackendRegistry`] for the
//! one matching a given language id.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use anyv_core::Paths;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A version pinned by some manifest the backend understands.
#[derive(Debug, Clone)]
pub struct DetectedVersion {
    pub version: String,
    /// Which file/source declared it (`go.mod`, `Gemfile`, …).
    pub source: String,
    pub origin: PathBuf,
}

/// What `Backend::install` returns.
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub version: String,
    pub install_dir: PathBuf,
    pub already_present: bool,
}

/// Optional vendor-/distribution-specific knobs threaded from the
/// manifest into `Backend::install`. Single-source backends ignore.
#[derive(Debug, Clone, Default)]
pub struct InstallOpts {
    /// Vendor selector (e.g. `"temurin"`, `"corretto"`, `"graalvm_community"`
    /// for Java). Backends that don't model multiple vendors ignore.
    pub distribution: Option<String>,
}

/// User-facing tool spec from `qusp.toml`. Either `"latest"`, an exact
/// version, or a constraint (`^v0.18`, `~1.64`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolSpec {
    Short(String),
    Long {
        #[serde(default)]
        package: Option<String>,
        version: String,
        #[serde(default)]
        bin: Option<String>,
    },
}

impl ToolSpec {
    pub fn version(&self) -> &str {
        match self {
            ToolSpec::Short(v) => v,
            ToolSpec::Long { version, .. } => version,
        }
    }
    pub fn package_override(&self) -> Option<&str> {
        match self {
            ToolSpec::Short(_) => None,
            ToolSpec::Long { package, .. } => package.as_deref(),
        }
    }
    pub fn bin_override(&self) -> Option<&str> {
        match self {
            ToolSpec::Short(_) => None,
            ToolSpec::Long { bin, .. } => bin.as_deref(),
        }
    }
}

/// Backend-resolved tool: spec + concrete version + canonical name.
#[derive(Debug, Clone)]
pub struct ResolvedTool {
    pub name: String,
    pub package: String,
    pub version: String,
    pub bin: String,
    /// Backend-specific opaque hash field (sumdb h1: for Go, gem sha for
    /// Ruby, etc.). Stored verbatim in the lock; not interpreted by core.
    pub upstream_hash: String,
}

/// What ends up in `qusp.lock` per tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedTool {
    pub name: String,
    pub package: String,
    pub version: String,
    pub bin: String,
    pub upstream_hash: String,
    pub built_with: String, // toolchain version that produced the binary
}

/// Environment to inject when running a command via this backend.
/// `qusp run` merges these across all relevant backends.
#[derive(Debug, Clone, Default)]
pub struct RunEnv {
    /// PATH entries to prepend, in order.
    pub path_prepend: Vec<PathBuf>,
    /// Additional env vars (GOROOT, GEM_PATH, RUBY_ROOT, etc.).
    pub env: BTreeMap<String, String>,
}

/// The trait every language backend implements.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Stable id used as the section name in `qusp.toml` (`go`, `ruby`,
    /// `python`, `node`, `terraform`, `deno`, `java`).
    fn id(&self) -> &'static str;

    /// Files this backend reads when walking up from cwd, in priority
    /// order. Used for project-root detection and version resolution.
    fn manifest_files(&self) -> &[&'static str];

    /// Synchronous "is this tool name in my registry?" check. Used by
    /// the orchestrator to route `qusp add tool <name>` to the right
    /// backend without firing slow network calls. Default `false` —
    /// backends with a known tool list override.
    fn knows_tool(&self, _name: &str) -> bool {
        false
    }

    /// Other backend ids this backend depends on at run time. Kotlin
    /// requires Java; Scala (future) requires Java; future Clojure
    /// requires Java. Default empty — most backends are self-contained.
    /// The orchestrator validates that every required backend is also
    /// pinned in the manifest before any install runs. Cross-backend
    /// envs merge automatically via `build_run_env` since the
    /// orchestrator already calls each pinned backend's env builder.
    fn requires(&self) -> &[&'static str] {
        &[]
    }

    /// Detect the version pinned by manifests. `Ok(None)` if no source
    /// pins one (caller falls through to global / latest installed).
    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>>;

    /// Install a toolchain version. Idempotent. `opts` carries
    /// vendor-specific knobs (distribution, etc.); backends that don't
    /// need them ignore via `let _ = opts;`.
    async fn install(
        &self,
        paths: &Paths,
        version: &str,
        opts: &InstallOpts,
    ) -> Result<InstallReport>;

    /// Drop a toolchain version (does not touch tool installs that
    /// depended on it; see cache prune).
    fn uninstall(&self, paths: &Paths, version: &str) -> Result<()>;

    /// List installed versions, newest first.
    fn list_installed(&self, paths: &Paths) -> Result<Vec<String>>;

    /// List all installable versions known to the upstream catalog.
    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>>;

    /// Resolve a tool spec to a concrete version + canonical metadata.
    async fn resolve_tool(
        &self,
        client: &reqwest::Client,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool>;

    /// Install a tool against the given toolchain version.
    async fn install_tool(
        &self,
        paths: &Paths,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool>;

    /// Where the installed binary lives. Used by `qusp run` and `qusp which`.
    fn tool_bin_path(&self, paths: &Paths, locked: &LockedTool) -> PathBuf;

    /// Build the env (PATH, language-specific vars) for `qusp run`.
    fn build_run_env(&self, paths: &Paths, version: &str, cwd: &Path) -> Result<RunEnv>;
}
