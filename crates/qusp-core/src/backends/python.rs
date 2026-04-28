//! Python backend — delegates everything to [uv](https://github.com/astral-sh/uv).
//!
//! Rationale: uv is best-in-class for Python. Reimplementing it inside qusp
//! would be a categorical mistake. Instead, qusp acts as an integrator
//! that records the intended Python version in `qusp.toml` and routes
//! every Python-related operation through `uv`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths;
use async_trait::async_trait;

use crate::backend::*;

pub struct PythonBackend;

const UV_BIN: &str = "uv";

fn uv_available() -> bool {
    which(UV_BIN).is_ok()
}

fn which(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(anyhow!("`{name}` not found on PATH"))
}

fn ensure_uv() -> Result<()> {
    if uv_available() {
        return Ok(());
    }
    Err(anyhow!(
        "qusp's python backend delegates to uv. Install with `brew install uv` \
         or `curl -LsSf https://astral.sh/uv/install.sh | sh`."
    ))
}

#[async_trait]
impl Backend for PythonBackend {
    fn id(&self) -> &'static str {
        "python"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["pyproject.toml", ".python-version"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        // Walk up looking for .python-version (uv & pyenv share this convention).
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".python-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).ok().unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".python-version".into(),
                        origin: f,
                    }));
                }
            }
            dir = d.parent();
        }
        Ok(None)
    }

    async fn install(&self, _paths: &Paths, version: &str) -> Result<InstallReport> {
        ensure_uv()?;
        let status = Command::new(UV_BIN)
            .args(["python", "install", version])
            .status()
            .with_context(|| format!("spawn uv python install {version}"))?;
        if !status.success() {
            bail!(
                "uv python install {version} failed (exit {:?})",
                status.code()
            );
        }
        Ok(InstallReport {
            version: version.to_string(),
            install_dir: PathBuf::new(), // uv-managed; we don't track its path
            already_present: false,
        })
    }

    fn uninstall(&self, _paths: &Paths, version: &str) -> Result<()> {
        ensure_uv()?;
        let status = Command::new(UV_BIN)
            .args(["python", "uninstall", version])
            .status()?;
        if !status.success() {
            bail!("uv python uninstall {version} failed");
        }
        Ok(())
    }

    fn list_installed(&self, _paths: &Paths) -> Result<Vec<String>> {
        if !uv_available() {
            return Ok(vec![]);
        }
        let out = Command::new(UV_BIN)
            .args(["python", "list", "--only-installed"])
            .output()?;
        if !out.status.success() {
            return Ok(vec![]);
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    async fn list_remote(&self, _client: &reqwest::Client) -> Result<Vec<String>> {
        ensure_uv()?;
        let out = Command::new(UV_BIN).args(["python", "list"]).output()?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    async fn resolve_tool(
        &self,
        _client: &reqwest::Client,
        _name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!(
            "Python tools are managed by uv directly. Use `uv tool install <name>` \
               or `uvx <name>`. qusp v0.1.0 will route `qusp tool add <python-tool>` to uv."
        )
    }

    async fn install_tool(
        &self,
        _paths: &Paths,
        _toolchain_version: &str,
        _resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        bail!("Python tool routing through uv arrives in v0.1.0.")
    }

    fn tool_bin_path(&self, _paths: &Paths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _paths: &Paths, _version: &str, _cwd: &Path) -> Result<RunEnv> {
        // uv handles its own env via `uv run`; we just exec uv when asked.
        Ok(RunEnv::default())
    }
}
