//! Multi-language orchestration. Fans out across registered backends in
//! parallel for installs, lock reconciliation, and tool routing.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use anyv_core::Paths;
use futures::future::try_join_all;

use crate::backend::{Backend, InstallOpts, LockedTool, ResolvedTool, ToolSpec};
use crate::lock::Lock;
use crate::manifest::Manifest;
use crate::registry::BackendRegistry;

pub struct Orchestrator<'a> {
    pub registry: &'a BackendRegistry,
    pub paths: &'a Paths,
}

#[derive(Debug, Clone)]
pub struct InstallSummary {
    pub lang: String,
    pub version: String,
    pub already_present: bool,
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub langs_installed: Vec<InstallSummary>,
    pub langs_failed: Vec<(String, String)>,
    pub tools_installed: Vec<(String, LockedTool)>,
    pub tools_removed_from_lock: usize,
}

#[derive(Debug, Clone)]
pub struct InstallToolchainsResult {
    pub installed: Vec<InstallSummary>,
    pub failed: Vec<(String, String)>,
}

impl<'a> Orchestrator<'a> {
    pub fn new(registry: &'a BackendRegistry, paths: &'a Paths) -> Self {
        Self { registry, paths }
    }

    /// Validate cross-backend requirements declared via `Backend::requires`.
    /// Errors before any install runs if a dependency is missing — e.g.
    /// `[kotlin]` pinned without `[java]` is caught here, not after a
    /// kotlin install gets halfway through.
    pub fn validate_requires(&self, manifest: &Manifest) -> Result<()> {
        for lang in manifest.languages.keys() {
            let Some(backend) = self.registry.get(lang) else {
                continue;
            };
            for required in backend.requires() {
                if !manifest.languages.contains_key(*required) {
                    bail!(
                        "[{lang}] requires [{required}] to be pinned in qusp.toml — \
                         add a [{required}] section with a version before installing {lang}"
                    );
                }
            }
        }
        Ok(())
    }

    /// Install every (lang, version) declared in the manifest. Runs in
    /// parallel across backends. Threads each section's `distribution`
    /// into the backend's `InstallOpts`. Per-backend failures are
    /// **collected, not propagated** — one broken backend doesn't kill
    /// the rest. The caller decides whether the partial set is OK.
    pub async fn install_toolchains(&self, manifest: &Manifest) -> Result<InstallToolchainsResult> {
        self.validate_requires(manifest)?;
        let mut futs = Vec::new();
        for (lang, sec) in &manifest.languages {
            let Some(version) = sec.version.clone() else {
                continue;
            };
            let Some(backend) = self.registry.get(lang) else {
                continue;
            };
            let paths = self.paths.clone();
            let lang = lang.clone();
            let opts = InstallOpts {
                distribution: sec.distribution.clone(),
            };
            futs.push(async move {
                let result = backend.install(&paths, &version, &opts).await;
                (lang, result)
            });
        }
        let outcomes = futures::future::join_all(futs).await;
        let mut installed = Vec::new();
        let mut failed = Vec::new();
        for (lang, result) in outcomes {
            match result {
                Ok(report) => installed.push(InstallSummary {
                    lang,
                    version: report.version,
                    already_present: report.already_present,
                }),
                Err(e) => failed.push((lang, format!("{e:#}"))),
            }
        }
        Ok(InstallToolchainsResult { installed, failed })
    }

    /// Install every pinned tool, in parallel. Requires that the relevant
    /// toolchain is already installed.
    pub async fn install_tools(
        &self,
        manifest: &Manifest,
        lock: &mut Lock,
        frozen: bool,
        client: &reqwest::Client,
    ) -> Result<Vec<(String, LockedTool)>> {
        // Collect (lang, name, spec) tuples first.
        let mut planned: Vec<(String, String, ToolSpec)> = Vec::new();
        for (lang, sec) in &manifest.languages {
            for (name, spec) in &sec.tools {
                planned.push((lang.clone(), name.clone(), spec.clone()));
            }
        }
        if planned.is_empty() {
            return Ok(vec![]);
        }

        // Resolve in parallel.
        let mut resolve_futs = Vec::new();
        for (lang, name, spec) in &planned {
            let Some(backend) = self.registry.get(lang) else {
                bail!("backend '{lang}' is not registered but appears in manifest");
            };
            let lang = lang.clone();
            let name = name.clone();
            if frozen {
                let prev = lock
                    .backends
                    .get(&lang)
                    .and_then(|b| b.tools.iter().find(|t| t.name == name).cloned())
                    .ok_or_else(|| {
                        anyhow!(
                        "frozen sync: {lang} tool '{name}' is in qusp.toml but not in qusp.lock"
                    )
                    })?;
                resolve_futs.push(Box::pin(async move {
                    Ok::<_, anyhow::Error>((
                        lang,
                        ResolvedTool {
                            name: prev.name,
                            package: prev.package,
                            version: prev.version,
                            bin: prev.bin,
                            upstream_hash: prev.upstream_hash,
                        },
                    ))
                })
                    as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>>);
            } else {
                let spec = spec.clone();
                let client = client.clone();
                resolve_futs.push(Box::pin(async move {
                    let r = backend.resolve_tool(&client, &name, &spec).await?;
                    Ok((lang, r))
                }));
            }
        }
        let resolved: Vec<(String, ResolvedTool)> = try_join_all(resolve_futs).await?;

        // Install in parallel.
        let mut install_futs = Vec::new();
        for (lang, r) in resolved {
            let Some(backend) = self.registry.get(&lang) else {
                continue;
            };
            let toolchain_version = manifest
                .languages
                .get(&lang)
                .and_then(|s| s.version.clone())
                .ok_or_else(|| anyhow!("no [{lang}] version pinned for tool '{}'", r.name))?;
            let paths = self.paths.clone();
            let r_clone = r.clone();
            let lang_clone = lang.clone();
            install_futs.push(async move {
                let locked = backend
                    .install_tool(&paths, &toolchain_version, &r_clone)
                    .await?;
                Ok::<_, anyhow::Error>((lang_clone, locked))
            });
        }
        let installed: Vec<(String, LockedTool)> = try_join_all(install_futs).await?;

        // Update lock.
        for (lang, locked) in &installed {
            let entry = lock.backends.entry(lang.clone()).or_default();
            if entry.version.is_empty() {
                if let Some(v) = manifest.languages.get(lang).and_then(|s| s.version.clone()) {
                    entry.version = v;
                }
            }
            entry.tools.retain(|t| t.name != locked.name);
            entry.tools.push(locked.clone());
            entry.tools.sort_by(|a, b| a.name.cmp(&b.name));
        }
        Ok(installed)
    }

    /// Drop tools that are in the lock but no longer pinned in the
    /// manifest. Returns the number removed.
    pub fn prune_stale_tools(&self, manifest: &Manifest, lock: &mut Lock) -> usize {
        let mut removed = 0usize;
        for (lang, entry) in lock.backends.iter_mut() {
            let pinned: std::collections::HashSet<&str> = manifest
                .languages
                .get(lang)
                .map(|s| s.tools.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            let before = entry.tools.len();
            entry.tools.retain(|t| pinned.contains(t.name.as_str()));
            removed += before - entry.tools.len();
        }
        removed
    }

    /// Refresh `LockedBackend.version` (and `distribution`, when set)
    /// for every language in the manifest so the lock's toolchain pins
    /// reflect the manifest after install.
    pub fn sync_toolchain_versions(&self, manifest: &Manifest, lock: &mut Lock) {
        for (lang, sec) in &manifest.languages {
            let Some(v) = sec.version.clone() else {
                continue;
            };
            let entry = lock.backends.entry(lang.clone()).or_default();
            entry.version = v;
            entry.distribution = sec.distribution.clone().unwrap_or_default();
        }
    }

    /// End-to-end sync: install toolchains, install tools, prune stale,
    /// reconcile lock. Toolchain install failures are surfaced in the
    /// summary instead of aborting the whole sync — a broken Python
    /// pin shouldn't block Go tools from installing.
    pub async fn sync(
        &self,
        manifest: &Manifest,
        lock: &mut Lock,
        frozen: bool,
        client: &reqwest::Client,
    ) -> Result<SyncSummary> {
        let install_result = self.install_toolchains(manifest).await?;
        self.sync_toolchain_versions(manifest, lock);
        let tools = self.install_tools(manifest, lock, frozen, client).await?;
        let removed = if !frozen {
            self.prune_stale_tools(manifest, lock)
        } else {
            0
        };
        Ok(SyncSummary {
            langs_installed: install_result.installed,
            langs_failed: install_result.failed,
            tools_installed: tools,
            tools_removed_from_lock: removed,
        })
    }

    /// Route a tool name to whichever backend's static registry knows it.
    /// Returns `(language_id, backend)`.
    pub fn route_tool(&self, name: &str) -> Result<(String, Arc<dyn Backend>)> {
        for (id, backend) in self.registry.iter() {
            if backend.knows_tool(name) {
                return Ok((id.to_string(), backend));
            }
        }
        bail!(
            "no backend recognized tool '{name}'. Pin it explicitly under \
             [<lang>.tools] in qusp.toml with package = \"…\"."
        )
    }

    /// Install + lock a single tool.
    pub async fn add_tool(
        &self,
        manifest: &mut Manifest,
        lock: &mut Lock,
        name: &str,
        version: &str,
        client: &reqwest::Client,
    ) -> Result<(String, LockedTool)> {
        let (lang, backend) = self.route_tool(name)?;
        let toolchain_version = manifest
            .languages
            .get(&lang)
            .and_then(|s| s.version.clone())
            .ok_or_else(|| {
                anyhow!(
                    "[{lang}] version is not pinned in qusp.toml — add it before installing tools"
                )
            })?;
        let spec = ToolSpec::Short(version.to_string());
        let resolved = backend.resolve_tool(client, name, &spec).await?;
        let locked = backend
            .install_tool(self.paths, &toolchain_version, &resolved)
            .await?;
        let distribution = manifest
            .languages
            .get(&lang)
            .and_then(|s| s.distribution.clone());
        // Update manifest.
        let sec = manifest.languages.entry(lang.clone()).or_default();
        sec.tools.insert(name.to_string(), spec);
        // Update lock.
        let entry = lock.backends.entry(lang.clone()).or_default();
        if entry.version.is_empty() {
            entry.version = toolchain_version.clone();
        }
        if entry.distribution.is_empty() {
            entry.distribution = distribution.unwrap_or_default();
        }
        entry.tools.retain(|t| t.name != locked.name);
        entry.tools.push(locked.clone());
        entry.tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((lang, locked))
    }

    /// Look up a tool by name across all backends' lock entries. Returns
    /// `(lang, locked, bin_path)` for the first match.
    pub fn find_tool(
        &self,
        lock: &Lock,
        name: &str,
    ) -> Option<(String, LockedTool, std::path::PathBuf)> {
        for (lang, entry) in &lock.backends {
            if let Some(t) = entry.tools.iter().find(|t| t.name == name) {
                let backend = self.registry.get(lang)?;
                let bin = backend.tool_bin_path(self.paths, t);
                return Some((lang.clone(), t.clone(), bin));
            }
        }
        None
    }

    /// Build a merged RunEnv across backends that have a pinned toolchain.
    /// Order: if `lang` is provided, that backend's env wins (front of PATH).
    /// Otherwise all backends' envs are concatenated.
    pub fn build_run_env(
        &self,
        lock: &Lock,
        cwd: &Path,
        prefer_lang: Option<&str>,
    ) -> Result<crate::backend::RunEnv> {
        let mut merged = crate::backend::RunEnv::default();
        let order: Vec<String> = if let Some(p) = prefer_lang {
            let mut o: Vec<String> = vec![p.to_string()];
            for id in self.registry.ids() {
                if id != p {
                    o.push(id.to_string());
                }
            }
            o
        } else {
            self.registry.ids().map(String::from).collect()
        };
        for lang in order {
            let Some(backend) = self.registry.get(&lang) else {
                continue;
            };
            let Some(entry) = lock.backends.get(&lang) else {
                continue;
            };
            if entry.version.is_empty() {
                continue;
            }
            let env = backend.build_run_env(self.paths, &entry.version, cwd)?;
            for p in env.path_prepend {
                merged.path_prepend.push(p);
            }
            for (k, v) in env.env {
                merged.env.insert(k, v);
            }
        }
        Ok(merged)
    }
}
