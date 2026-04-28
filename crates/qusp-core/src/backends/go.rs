//! Go backend, v0.0.1 — wraps `gv` as a subprocess. v0.1.0 will pull
//! `gv-core` in directly so we don't depend on a separate `gv` install.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::Paths;
use async_trait::async_trait;

use crate::backend::*;

pub struct GoBackend;

const GV_BIN: &str = "gv";

fn gv_available() -> bool {
    which("gv").is_ok()
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

#[async_trait]
impl Backend for GoBackend {
    fn id(&self) -> &'static str {
        "go"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["go.mod", ".go-version"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        // Reuse `gv current` — handles go.mod toolchain + .go-version.
        if !gv_available() {
            return Ok(None);
        }
        let out = Command::new(GV_BIN)
            .arg("current")
            .current_dir(cwd)
            .output();
        let Ok(out) = out else {
            return Ok(None);
        };
        if !out.status.success() {
            return Ok(None);
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut lines = stdout.lines();
        let Some(version) = lines.next() else {
            return Ok(None);
        };
        let version = version.trim();
        if version.is_empty() {
            return Ok(None);
        }
        let source = lines
            .next()
            .map(|l| l.trim().trim_start_matches("source: ").to_string())
            .unwrap_or_else(|| "gv".into());
        Ok(Some(DetectedVersion {
            version: version.to_string(),
            source,
            origin: cwd.to_path_buf(),
        }))
    }

    async fn install(&self, _paths: &Paths, version: &str) -> Result<InstallReport> {
        ensure_gv()?;
        let status = Command::new(GV_BIN)
            .args(["install", version])
            .status()
            .with_context(|| format!("spawn {GV_BIN} install {version}"))?;
        if !status.success() {
            bail!("gv install {version} failed (exit {:?})", status.code());
        }
        // Ask gv where it put the install.
        let out = Command::new(GV_BIN)
            .args(["dir", "versions"])
            .output()
            .context("spawn gv dir versions")?;
        let install_dir = if out.status.success() {
            let base = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // gv uses "go1.26.2" naming; normalize.
            let v = if version.starts_with("go") {
                version.to_string()
            } else {
                format!("go{version}")
            };
            PathBuf::from(base).join(v)
        } else {
            PathBuf::new()
        };
        Ok(InstallReport {
            version: version.to_string(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _paths: &Paths, version: &str) -> Result<()> {
        ensure_gv()?;
        let status = Command::new(GV_BIN).args(["uninstall", version]).status()?;
        if !status.success() {
            bail!("gv uninstall {version} failed");
        }
        Ok(())
    }

    fn list_installed(&self, _paths: &Paths) -> Result<Vec<String>> {
        if !gv_available() {
            return Ok(vec![]);
        }
        let out = Command::new(GV_BIN).arg("list").output()?;
        if !out.status.success() {
            return Ok(vec![]);
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| l.starts_with("go"))
            .collect())
    }

    async fn list_remote(&self, _client: &reqwest::Client) -> Result<Vec<String>> {
        ensure_gv()?;
        let out = Command::new(GV_BIN).args(["list", "--remote"]).output()?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().nth(1).map(|s| s.to_string()))
            .collect())
    }

    async fn resolve_tool(
        &self,
        _client: &reqwest::Client,
        _name: &str,
        _spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        bail!("qusp v0.0.1 does not yet route Go tool installs through `gv`. Coming in v0.1.0.")
    }

    async fn install_tool(
        &self,
        _paths: &Paths,
        _toolchain_version: &str,
        _resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        bail!("qusp v0.0.1 does not yet route Go tool installs through `gv`. Coming in v0.1.0.")
    }

    fn tool_bin_path(&self, _paths: &Paths, locked: &LockedTool) -> PathBuf {
        // Placeholder until v0.1.0; gv's own `which` would be the right thing.
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _paths: &Paths, _version: &str, _cwd: &Path) -> Result<RunEnv> {
        // gv shim handles env on its own; nothing to inject here for now.
        Ok(RunEnv::default())
    }
}

fn ensure_gv() -> Result<()> {
    if gv_available() {
        return Ok(());
    }
    Err(anyhow!(
        "qusp's go backend currently delegates to `gv` (https://github.com/O6lvl4/gv). \
         Install it with `brew install O6lvl4/tap/gv` or `cargo install --git https://github.com/O6lvl4/gv gv-cli`."
    ))
}
