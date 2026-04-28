//! Ruby backend — uses [`rv-core`](https://github.com/O6lvl4/rv) as a Cargo
//! dependency. Compilation goes through `ruby-build` (a system prereq;
//! `brew install ruby-build`), the de-facto compile script every Ruby
//! version manager (rbenv/asdf/mise/rv) has used for a decade.

use std::path::{Path, PathBuf};

use anyhow::Result;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;

use crate::backend::*;

pub struct RubyBackend;

#[async_trait]
impl Backend for RubyBackend {
    fn id(&self) -> &'static str {
        "ruby"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["Gemfile", ".ruby-version"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let paths = rv_core::paths::discover()?;
        match rv_core::resolve::resolve(&paths, cwd)? {
            Some(r) => Ok(Some(DetectedVersion {
                version: r.version,
                source: format!("{:?}", r.source).to_lowercase(),
                origin: r.origin.unwrap_or_else(|| cwd.to_path_buf()),
            })),
            None => Ok(None),
        }
    }

    async fn install(&self, _qusp_paths: &AnyvPaths, version: &str) -> Result<InstallReport> {
        let paths = rv_core::paths::discover()?;
        paths.ensure_dirs()?;
        let report = tokio::task::spawn_blocking({
            let paths = paths.clone();
            let v = version.to_string();
            move || rv_core::install::install(&paths, &v)
        })
        .await
        .map_err(|e| anyhow::anyhow!("ruby-build task panicked: {e}"))??;
        Ok(InstallReport {
            version: report.version,
            install_dir: report.install_dir,
            already_present: report.already_present,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = rv_core::paths::discover()?;
        rv_core::install::uninstall(&paths, version)
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = rv_core::paths::discover()?;
        rv_core::resolve::list_installed(&paths)
    }

    async fn list_remote(&self, _client: &reqwest::Client) -> Result<Vec<String>> {
        // Delegates to ruby-build --definitions (synchronous shell-out).
        tokio::task::spawn_blocking(rv_core::install::list_remote)
            .await
            .map_err(|e| anyhow::anyhow!("list_remote task panicked: {e}"))?
    }

    async fn resolve_tool(
        &self,
        client: &reqwest::Client,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        let rv_spec = match spec {
            ToolSpec::Short(v) => rv_core::project::ToolSpec::Short(v.clone()),
            ToolSpec::Long {
                package,
                version,
                bin,
            } => rv_core::project::ToolSpec::Long {
                gem: package.clone(),
                version: version.clone(),
                bin: bin.clone(),
            },
        };
        let r = rv_core::tool::resolve(client, name, &rv_spec).await?;
        Ok(ResolvedTool {
            name: r.name,
            package: r.gem,
            version: r.version,
            bin: r.bin,
            upstream_hash: r.gem_sha256,
        })
    }

    async fn install_tool(
        &self,
        _qusp_paths: &AnyvPaths,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        let paths = rv_core::paths::discover()?;
        let rv_resolved = rv_core::tool::ResolvedTool {
            name: resolved.name.clone(),
            gem: resolved.package.clone(),
            version: resolved.version.clone(),
            bin: resolved.bin.clone(),
            gem_sha256: resolved.upstream_hash.clone(),
        };
        let r = tokio::task::spawn_blocking({
            let paths = paths.clone();
            let v = toolchain_version.to_string();
            move || rv_core::tool::install(&paths, &v, &rv_resolved)
        })
        .await
        .map_err(|e| anyhow::anyhow!("gem install task panicked: {e}"))??;
        Ok(LockedTool {
            name: r.name,
            package: r.gem,
            version: r.version,
            bin: r.bin,
            upstream_hash: r.gem_sha256,
            built_with: r.built_with,
        })
    }

    fn tool_bin_path(&self, _qusp_paths: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        let paths = match rv_core::paths::discover() {
            Ok(p) => p,
            Err(_) => return PathBuf::from(&locked.bin),
        };
        let rv_locked = rv_core::lock::LockedTool {
            name: locked.name.clone(),
            gem: locked.package.clone(),
            version: locked.version.clone(),
            bin: locked.bin.clone(),
            gem_sha256: locked.upstream_hash.clone(),
            built_with: locked.built_with.clone(),
        };
        rv_core::tool::tool_bin_path(&paths, &rv_locked)
    }

    fn build_run_env(&self, _qusp_paths: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = rv_core::paths::discover()?;
        let ruby_root = paths.version_dir(version);
        Ok(RunEnv {
            path_prepend: vec![ruby_root.join("bin")],
            env: Default::default(),
        })
    }
}
