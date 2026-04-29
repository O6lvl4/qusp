//! Multi-language orchestration. Fans out across registered backends in
//! parallel for installs, lock reconciliation, and tool routing.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use anyv_core::Paths;
use futures::future::try_join_all;

use crate::backend::{Backend, InstallOpts, LockedTool, ResolvedTool, ToolSpec};
use crate::domain::plan::{plan_install_toolchains, plan_sync, InstallPlan, SyncPlan};
use crate::domain::PinnedManifest;
use crate::effects::{HttpFetcher, LiveHttp};
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

    /// High-level: plan + execute. Composes `plan_install_toolchains`
    /// (pure) and `execute_install_plans` (effect). Most callers want
    /// this; tests / `qusp plan` get to inspect the plan before
    /// execution by calling the parts separately.
    pub async fn install_toolchains(
        &self,
        manifest: &PinnedManifest,
    ) -> Result<InstallToolchainsResult> {
        let plans = plan_install_toolchains(manifest);
        self.execute_install_plans(&plans).await
    }

    /// **Effect:** run the given install plans in parallel across
    /// backends. Threads each plan's distribution into the backend's
    /// `InstallOpts`. Per-backend failures are **collected, not
    /// propagated** — one broken backend doesn't kill the rest. The
    /// caller decides whether the partial set is OK.
    pub async fn execute_install_plans(
        &self,
        plans: &[InstallPlan],
    ) -> Result<InstallToolchainsResult> {
        let mut futs = Vec::new();
        for plan in plans {
            let Some(backend) = self.registry.get(plan.language.as_str()) else {
                continue;
            };
            let paths = self.paths.clone();
            let lang = plan.language.as_str().to_string();
            let version = plan.version.as_str().to_string();
            let opts = InstallOpts {
                distribution: plan.distribution.as_ref().map(|d| d.as_str().to_string()),
            };
            futs.push(async move {
                let http = LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))
                    .expect("LiveHttp build cannot fail with default reqwest config");
                let progress = crate::effects::LiveProgress::new();
                let result = backend
                    .install(&paths, &version, &opts, &http, &progress)
                    .await;
                (lang, version, result)
            });
        }
        let outcomes = futures::future::join_all(futs).await;
        let mut installed = Vec::new();
        let mut failed = Vec::new();
        // Load global pins once for the post-install farm pass.
        let global_pins = crate::effects::GlobalPins::load(&self.paths.config)
            .unwrap_or_default();
        let farm = crate::effects::FarmManager::default();
        let store_root = self.paths.store();
        for (lang, version, result) in outcomes {
            match result {
                Ok(report) => {
                    // Materialise farm symlinks. Versioned binaries
                    // unconditionally; unversioned only when the user's
                    // global pin says this version is the bare-command
                    // owner. Failures are non-fatal — the install
                    // succeeded, the farm is a UX add-on.
                    if !report.already_present {
                        let backend = self.registry.get(&lang);
                        if let Some(backend) = backend {
                            let bins = backend.farm_binaries(&version);
                            if !bins.is_empty() {
                                let pin_matches = global_pins
                                    .get(&lang)
                                    .map(|p| p.version == version)
                                    .unwrap_or(false);
                                if let Err(e) = farm.install_links(
                                    &report.install_dir,
                                    &bins,
                                    pin_matches,
                                    &store_root,
                                ) {
                                    tracing::warn!(
                                        "farm: link install failed for {lang} {version}: {e:#}"
                                    );
                                }
                            }
                        }
                    }
                    installed.push(InstallSummary {
                        lang,
                        version: report.version,
                        already_present: report.already_present,
                    });
                }
                Err(e) => failed.push((lang, format!("{e:#}"))),
            }
        }
        Ok(InstallToolchainsResult { installed, failed })
    }

    /// High-level sync: plan + execute. Composes `plan_sync` (pure)
    /// and `execute_sync_plan` (effect). Toolchain install failures
    /// are surfaced in the summary instead of aborting — a broken
    /// Python pin shouldn't block Go tools from installing.
    pub async fn sync(
        &self,
        manifest: &PinnedManifest,
        lock: &mut Lock,
        frozen: bool,
        http: &dyn HttpFetcher,
    ) -> Result<SyncSummary> {
        let plan = plan_sync(manifest, lock, frozen)?;
        self.execute_sync_plan(&plan, lock, http).await
    }

    /// **Effect:** apply a SyncPlan against the live system + the
    /// mutable lock. The plan itself is pure; this method is where
    /// HTTP, filesystem writes, and lock mutations happen.
    pub async fn execute_sync_plan(
        &self,
        plan: &SyncPlan,
        lock: &mut Lock,
        http: &dyn HttpFetcher,
    ) -> Result<SyncSummary> {
        let install_result = self.execute_install_plans(&plan.install_toolchains).await?;
        self.apply_lock_header_updates(plan, lock);
        let tools = self.execute_tool_install_plans(plan, lock, http).await?;
        let removed = self.apply_tool_prunes(plan, lock);
        Ok(SyncSummary {
            langs_installed: install_result.installed,
            langs_failed: install_result.failed,
            tools_installed: tools,
            tools_removed_from_lock: removed,
        })
    }

    /// **Effect:** write the plan's lock header updates into the lock.
    fn apply_lock_header_updates(&self, plan: &SyncPlan, lock: &mut Lock) {
        for upd in &plan.lock_header_updates {
            let entry = lock
                .backends
                .entry(upd.language.as_str().to_string())
                .or_default();
            entry.version = upd.version.as_str().to_string();
            entry.distribution = upd
                .distribution
                .as_ref()
                .map(|d| d.as_str().to_string())
                .unwrap_or_default();
        }
    }

    /// **Effect:** drop the plan's pruned tools from the lock; return
    /// how many were removed.
    fn apply_tool_prunes(&self, plan: &SyncPlan, lock: &mut Lock) -> usize {
        let mut removed = 0;
        for prune in &plan.prune_tools {
            if let Some(entry) = lock.backends.get_mut(prune.language.as_str()) {
                let before = entry.tools.len();
                entry.tools.retain(|t| t.name != prune.tool_name);
                removed += before - entry.tools.len();
            }
        }
        removed
    }

    /// **Effect:** resolve + install every tool described by the plan.
    /// Frozen entries skip resolve and reuse the lock's previous
    /// LockedTool verbatim.
    async fn execute_tool_install_plans(
        &self,
        plan: &SyncPlan,
        lock: &mut Lock,
        _http: &dyn HttpFetcher,
    ) -> Result<Vec<(String, LockedTool)>> {
        if plan.install_tools.is_empty() {
            return Ok(vec![]);
        }

        // Resolve in parallel.
        let mut resolve_futs = Vec::new();
        for tool in &plan.install_tools {
            let Some(backend) = self.registry.get(tool.language.as_str()) else {
                bail!(
                    "backend '{}' is not registered but appears in plan",
                    tool.language
                );
            };
            let lang = tool.language.as_str().to_string();
            let name = tool.tool_name.clone();
            if let Some(prev) = tool.frozen_carryover.clone() {
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
                let spec = tool.spec.clone();
                resolve_futs.push(Box::pin(async move {
                    let http = LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))?;
                    let r = backend.resolve_tool(&http, &name, &spec).await?;
                    Ok((lang, r))
                }));
            }
        }
        let resolved: Vec<(String, ResolvedTool)> = try_join_all(resolve_futs).await?;

        // Install in parallel.
        let mut install_futs = Vec::new();
        for ((lang, r), plan) in resolved.iter().zip(plan.install_tools.iter()) {
            let Some(backend) = self.registry.get(lang) else {
                continue;
            };
            let toolchain_version = plan.toolchain_version.as_str().to_string();
            let paths = self.paths.clone();
            let r_clone = r.clone();
            let lang_clone = lang.clone();
            install_futs.push(async move {
                let http = LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))?;
                let locked = backend
                    .install_tool(&paths, &http, &toolchain_version, &r_clone)
                    .await?;
                Ok::<_, anyhow::Error>((lang_clone, locked))
            });
        }
        let installed: Vec<(String, LockedTool)> = try_join_all(install_futs).await?;

        // Update lock entries with installed tools.
        for (lang, locked) in &installed {
            let entry = lock.backends.entry(lang.clone()).or_default();
            entry.tools.retain(|t| t.name != locked.name);
            entry.tools.push(locked.clone());
            entry.tools.sort_by(|a, b| a.name.cmp(&b.name));
        }
        Ok(installed)
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

    /// Install + lock a single tool. Mutates the **raw** `Manifest`
    /// because a successful add_tool needs to write the new pin back
    /// to qusp.toml; the caller can re-validate to a `PinnedManifest`
    /// after if needed.
    pub async fn add_tool(
        &self,
        manifest: &mut Manifest,
        lock: &mut Lock,
        name: &str,
        version: &str,
        http: &dyn HttpFetcher,
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
        let resolved = backend.resolve_tool(http, name, &spec).await?;
        let locked = backend
            .install_tool(self.paths, http, &toolchain_version, &resolved)
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
