//! Go backend — uses [`gv-core`](https://github.com/O6lvl4/gv) as a Cargo
//! dependency. Toolchain installs go directly to gv's content-addressed
//! store, so `qusp install go 1.26.2` and `gv install 1.26.2` produce
//! identical on-disk state.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;

use crate::backend::*;

pub struct GoBackend;

#[async_trait]
impl Backend for GoBackend {
    fn id(&self) -> &'static str {
        "go"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &["go.mod", ".go-version"]
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let paths = gv_core::paths::discover()?;
        match gv_core::resolve::resolve(&paths, cwd)? {
            Some(r) => Ok(Some(DetectedVersion {
                version: r.version,
                source: format!("{:?}", r.source).to_lowercase(),
                origin: r.origin.unwrap_or_else(|| cwd.to_path_buf()),
            })),
            None => Ok(None),
        }
    }

    async fn install(&self, _qusp_paths: &AnyvPaths, version: &str) -> Result<InstallReport> {
        let paths = gv_core::paths::discover()?;
        paths.ensure_dirs()?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("qusp-go/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let platform = gv_core::Platform::detect()?;
        let normalized = gv_core::release::normalize_version(version);
        let installer = gv_core::install::Installer {
            paths: &paths,
            client: &client,
            platform,
        };
        let report = installer.install(&normalized).await?;
        Ok(InstallReport {
            version: report.version,
            install_dir: report.install_dir,
            already_present: report.already_present,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = gv_core::paths::discover()?;
        let canonical = gv_core::release::normalize_version(version);
        let link = paths.version_dir(&canonical);
        if !link.exists() && !link.is_symlink() {
            anyhow::bail!("{canonical} is not installed");
        }
        std::fs::remove_file(&link)
            .or_else(|_| std::fs::remove_dir_all(&link))
            .with_context(|| format!("remove {}", link.display()))?;
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = gv_core::paths::discover()?;
        gv_core::resolve::list_installed(&paths)
    }

    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>> {
        let releases = gv_core::release::fetch_index(client).await?;
        Ok(releases.iter().map(|r| r.version.clone()).collect())
    }

    async fn resolve_tool(
        &self,
        client: &reqwest::Client,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        let gv_spec = match spec {
            ToolSpec::Short(v) => gv_core::project::ToolSpec::Short(v.clone()),
            ToolSpec::Long {
                package,
                version,
                bin,
            } => gv_core::project::ToolSpec::Long {
                package: package.clone(),
                version: version.clone(),
                bin: bin.clone(),
            },
        };
        let r = gv_core::tool::resolve(client, name, &gv_spec).await?;
        Ok(ResolvedTool {
            name: r.name,
            package: r.package,
            version: r.version,
            bin: r.bin,
            upstream_hash: r.module_hash,
        })
    }

    async fn install_tool(
        &self,
        _qusp_paths: &AnyvPaths,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        let paths = gv_core::paths::discover()?;
        let gv_resolved = gv_core::tool::ResolvedTool {
            name: resolved.name.clone(),
            package: resolved.package.clone(),
            version: resolved.version.clone(),
            bin: resolved.bin.clone(),
            module_hash: resolved.upstream_hash.clone(),
        };
        let r = tokio::task::spawn_blocking({
            let paths = paths.clone();
            let v = toolchain_version.to_string();
            move || gv_core::tool::install(&paths, &v, &gv_resolved)
        })
        .await
        .map_err(|e| anyhow::anyhow!("install task panicked: {e}"))??;
        Ok(LockedTool {
            name: r.name,
            package: r.package,
            version: r.version,
            bin: r.bin,
            upstream_hash: r.module_hash,
            built_with: r.built_with,
        })
    }

    fn tool_bin_path(&self, _qusp_paths: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        let paths = match gv_core::paths::discover() {
            Ok(p) => p,
            Err(_) => return PathBuf::from(&locked.bin),
        };
        let gv_locked = gv_core::lock::LockedTool {
            name: locked.name.clone(),
            package: locked.package.clone(),
            version: locked.version.clone(),
            bin: locked.bin.clone(),
            module_hash: locked.upstream_hash.clone(),
            built_with: locked.built_with.clone(),
            binary_sha256: String::new(),
        };
        gv_core::tool::tool_bin_path(&paths, &gv_locked)
    }

    fn build_run_env(&self, _qusp_paths: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        let paths = gv_core::paths::discover()?;
        let goroot = paths.version_dir(version);
        let mut env = std::collections::BTreeMap::new();
        env.insert("GOROOT".into(), goroot.to_string_lossy().into_owned());
        env.insert("GOTOOLCHAIN".into(), "local".into());
        Ok(RunEnv {
            path_prepend: vec![goroot.join("bin")],
            env,
        })
    }
}
