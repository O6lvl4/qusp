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

    /// **Effect:** run the given install plans, ordered into dependency
    /// layers by each backend's `requires()` so install-time cross-backend
    /// deps are satisfied (e.g. Erlang installs before Elixir, which reads
    /// the installed OTP major at install time). Backends within a layer
    /// have no inter-dependencies and install in parallel; layers run in
    /// sequence. Threads each plan's distribution into the backend's
    /// `InstallOpts`. Per-backend failures are **collected, not
    /// propagated** — one broken backend doesn't kill the rest; a backend
    /// whose required dependency failed is skipped (and recorded failed).
    /// The caller decides whether the partial set is OK.
    pub async fn execute_install_plans(
        &self,
        plans: &[InstallPlan],
    ) -> Result<InstallToolchainsResult> {
        let layers = layer_install_plans(plans, self.registry);

        // The set of langs being installed in *this* call — only deps
        // within it gate ordering; deps outside it are presumed already
        // installed (or were rejected by manifest validation).
        let in_set: std::collections::BTreeSet<&str> =
            plans.iter().map(|p| p.language.as_str()).collect();

        let mut installed = Vec::new();
        let mut failed: Vec<(String, String)> = Vec::new();
        let mut failed_langs: std::collections::BTreeSet<String> = Default::default();
        // Load global pins once for the post-install farm pass.
        let global_pins = crate::effects::GlobalPins::load(&self.paths.config).unwrap_or_default();
        let farm = crate::effects::FarmManager::default();
        let store_root = self.paths.store();

        for layer in layers {
            let mut futs = Vec::new();
            for plan in layer {
                let Some(backend) = self.registry.get(plan.language.as_str()) else {
                    continue;
                };
                let lang = plan.language.as_str().to_string();
                // Skip if an in-set requirement failed in an earlier layer.
                let unmet: Vec<&str> = backend
                    .requires()
                    .iter()
                    .copied()
                    .filter(|r| in_set.contains(r) && failed_langs.contains(*r))
                    .collect();
                if !unmet.is_empty() {
                    failed_langs.insert(lang.clone());
                    failed.push((
                        lang,
                        format!(
                            "skipped: required toolchain(s) failed to install: {}",
                            unmet.join(", ")
                        ),
                    ));
                    continue;
                }
                let paths = self.paths.clone();
                let version = plan.version.as_str().to_string();
                let opts = InstallOpts {
                    distribution: plan.distribution.as_ref().map(|d| d.as_str().to_string()),
                };
                futs.push(async move {
                    let http = LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))
                        .expect("LiveHttp build cannot fail with default reqwest config");
                    let progress = crate::effects::LiveProgress::new();
                    let ctx = crate::backend::InstallCtx {
                        opts: &opts,
                        http: &http,
                        progress: &progress,
                    };
                    let result = backend.install(&paths, &version, &ctx).await;
                    (lang, version, result)
                });
            }
            let outcomes = futures::future::join_all(futs).await;
            for (lang, version, result) in outcomes {
                match result {
                    Ok(report) => {
                        if !report.already_present {
                            Self::materialize_farm(
                                self.registry,
                                &lang,
                                &version,
                                &report,
                                &global_pins,
                                &farm,
                                &store_root,
                            );
                        }
                        installed.push(InstallSummary {
                            lang,
                            version: report.version,
                            already_present: report.already_present,
                        });
                    }
                    Err(e) => {
                        failed_langs.insert(lang.clone());
                        failed.push((lang, format!("{e:#}")));
                    }
                }
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

    fn materialize_farm(
        registry: &BackendRegistry,
        lang: &str,
        version: &str,
        report: &crate::backend::InstallReport,
        global_pins: &crate::effects::GlobalPins,
        farm: &crate::effects::FarmManager,
        store_root: &std::path::Path,
    ) {
        let Some(backend) = registry.get(lang) else {
            return;
        };
        let bins = backend.farm_binaries(version);
        if bins.is_empty() {
            return;
        }
        let pin_matches = global_pins
            .get(lang)
            .map(|p| p.version == version)
            .unwrap_or(false);
        if let Err(e) = farm.install_links(&report.install_dir, &bins, pin_matches, store_root) {
            tracing::warn!("farm: link install failed for {lang} {version}: {e:#}");
        }
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
        let order = self.lang_order(prefer_lang);
        for lang in &order {
            self.merge_backend_env(lock, lang, cwd, &mut merged)?;
        }
        Ok(merged)
    }

    fn lang_order(&self, prefer: Option<&str>) -> Vec<String> {
        match prefer {
            Some(p) => {
                let mut o = vec![p.to_string()];
                o.extend(self.registry.ids().filter(|id| *id != p).map(String::from));
                o
            }
            None => self.registry.ids().map(String::from).collect(),
        }
    }

    fn merge_backend_env(
        &self,
        lock: &Lock,
        lang: &str,
        cwd: &Path,
        merged: &mut crate::backend::RunEnv,
    ) -> Result<()> {
        let Some(backend) = self.registry.get(lang) else {
            return Ok(());
        };
        let Some(entry) = lock.backends.get(lang) else {
            return Ok(());
        };
        if entry.version.is_empty() {
            return Ok(());
        }
        let env = backend.build_run_env(self.paths, &entry.version, cwd)?;
        merged.path_prepend.extend(env.path_prepend);
        merged.env.extend(env.env);
        Ok(())
    }
}

/// Order install plans into dependency layers using each backend's
/// `requires()`. A plan lands in the earliest layer where all of its
/// in-set requirements already sit in an earlier layer; requirements not
/// present in `plans` are treated as already-satisfied. Within a layer
/// there are no inter-dependencies, so a layer installs in parallel.
///
/// Returns owned clones (small structs) so the executor can move them
/// into per-backend futures. If a dependency cycle makes progress
/// impossible, the remaining plans are emitted as one final layer rather
/// than looping forever (install order among them is then arbitrary).
fn layer_install_plans(plans: &[InstallPlan], registry: &BackendRegistry) -> Vec<Vec<InstallPlan>> {
    use std::collections::BTreeSet;
    let in_set: BTreeSet<&str> = plans.iter().map(|p| p.language.as_str()).collect();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    let mut remaining: Vec<&InstallPlan> = plans.iter().collect();
    let mut layers: Vec<Vec<InstallPlan>> = Vec::new();

    while !remaining.is_empty() {
        let (ready, blocked): (Vec<&InstallPlan>, Vec<&InstallPlan>) =
            remaining.iter().partition(|p| {
                registry
                    .get(p.language.as_str())
                    .map(|b| {
                        b.requires()
                            .iter()
                            .filter(|r| in_set.contains(*r))
                            .all(|r| placed.contains(*r))
                    })
                    .unwrap_or(true)
            });

        if ready.is_empty() {
            // Unsatisfiable (cycle) — emit the rest in one layer.
            layers.push(blocked.iter().map(|p| (*p).clone()).collect());
            break;
        }
        for p in &ready {
            placed.insert(p.language.as_str().to_string());
        }
        layers.push(ready.iter().map(|p| (*p).clone()).collect());
        remaining = blocked;
    }
    layers
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::backends;
    use crate::domain::pinned::validate;
    use crate::manifest::{LanguageSection, Manifest as RawManifest};

    fn registry() -> BackendRegistry {
        let mut r = BackendRegistry::new();
        r.register(Arc::new(backends::erlang::ErlangBackend));
        r.register(Arc::new(backends::elixir::ElixirBackend));
        r.register(Arc::new(backends::go::GoBackend));
        r
    }

    fn pinned(entries: &[(&str, &str)]) -> PinnedManifest {
        let mut languages: BTreeMap<String, LanguageSection> = BTreeMap::new();
        for (lang, version) in entries {
            languages.insert(
                (*lang).to_string(),
                LanguageSection {
                    version: Some((*version).to_string()),
                    distribution: None,
                    tools: BTreeMap::new(),
                },
            );
        }
        validate(&RawManifest { languages }, &registry()).unwrap()
    }

    /// Elixir (`requires = ["erlang"]`) must land in a strictly later
    /// layer than Erlang so its install-time OTP-major probe succeeds.
    #[test]
    fn elixir_layers_after_erlang() {
        let manifest = pinned(&[("elixir", "1.18.4"), ("erlang", "28.0"), ("go", "1.26.2")]);
        let plans = plan_install_toolchains(&manifest);
        let layers = layer_install_plans(&plans, &registry());

        let layer_of = |lang: &str| {
            layers
                .iter()
                .position(|l| l.iter().any(|p| p.language.as_str() == lang))
                .unwrap()
        };
        assert!(
            layer_of("erlang") < layer_of("elixir"),
            "erlang must install before elixir"
        );
        // go has no deps → first layer, alongside erlang.
        assert_eq!(layer_of("go"), layer_of("erlang"));
    }

    /// Without a dependent, every plan is independent → a single layer.
    #[test]
    fn independent_plans_share_one_layer() {
        let manifest = pinned(&[("erlang", "28.0"), ("go", "1.26.2")]);
        let plans = plan_install_toolchains(&manifest);
        let layers = layer_install_plans(&plans, &registry());
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].len(), 2);
    }
}
