//! Pure plan generation.
//!
//! These functions consume a [`PinnedManifest`] (and optionally a
//! `Lock`) and produce a description of what will happen. They make
//! no network calls, do no IO, and are trivially unit-testable.
//!
//! The orchestrator's `execute_*` functions take these plans and run
//! them. The split is the central premise of the Functional-DDD
//! migration: "decide what to do" lives in pure code; "do it" lives
//! in effect code; never mix.

use crate::backend::{LockedTool, ToolSpec};
use crate::domain::error::PlanError;
use crate::domain::pinned::PinnedManifest;
use crate::domain::types::{Distribution, LanguageId, Version};
use crate::lock::Lock;

/// One toolchain to install.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub language: LanguageId,
    pub version: Version,
    pub distribution: Option<Distribution>,
}

/// One tool to install (or carry over from an existing lock when
/// `frozen` is true).
#[derive(Debug, Clone)]
pub struct ToolInstallPlan {
    pub language: LanguageId,
    pub toolchain_version: Version,
    pub tool_name: String,
    pub spec: ToolSpec,
    /// When `Some`, plan is to carry the lock entry over verbatim
    /// (`--frozen` reuse). When `None`, plan is to resolve via the
    /// backend's `resolve_tool`.
    pub frozen_carryover: Option<LockedTool>,
}

/// One stale tool to drop from the lock.
#[derive(Debug, Clone)]
pub struct ToolPrunePlan {
    pub language: LanguageId,
    pub tool_name: String,
}

/// Lock metadata reconciliation.
#[derive(Debug, Clone)]
pub struct LockHeaderUpdate {
    pub language: LanguageId,
    pub version: Version,
    pub distribution: Option<Distribution>,
}

/// Aggregate of everything `qusp sync` will do.
#[derive(Debug, Clone)]
pub struct SyncPlan {
    pub install_toolchains: Vec<InstallPlan>,
    pub install_tools: Vec<ToolInstallPlan>,
    pub prune_tools: Vec<ToolPrunePlan>,
    pub lock_header_updates: Vec<LockHeaderUpdate>,
    pub frozen: bool,
}

/// Pure: which toolchains does `qusp install` want to install?
pub fn plan_install_toolchains(manifest: &PinnedManifest) -> Vec<InstallPlan> {
    manifest
        .iter()
        .map(|(lang, sec)| InstallPlan {
            language: lang.clone(),
            version: sec.version.clone(),
            distribution: sec.distribution.clone(),
        })
        .collect()
}

/// Pure: full sync plan. Returns an error only when `frozen` is set
/// and a manifest tool can't be carried over from the lock.
pub fn plan_sync(
    manifest: &PinnedManifest,
    lock: &Lock,
    frozen: bool,
) -> Result<SyncPlan, PlanError> {
    let install_toolchains = plan_install_toolchains(manifest);

    // Tools: every (lang, tool) pair pinned in the manifest gets a
    // ToolInstallPlan. In frozen mode, missing-from-lock errors out;
    // otherwise the carryover stays None and the executor will resolve
    // fresh via the backend.
    let mut install_tools = Vec::new();
    for (lang, sec) in manifest.iter() {
        for (name, spec) in &sec.tools {
            let frozen_carryover = if frozen {
                let prev = lock
                    .backends
                    .get(lang.as_str())
                    .and_then(|b| b.tools.iter().find(|t| &t.name == name).cloned())
                    .ok_or_else(|| PlanError::FrozenToolMissingFromLock {
                        lang: lang.as_str().to_string(),
                        tool: name.clone(),
                    })?;
                Some(prev)
            } else {
                None
            };
            install_tools.push(ToolInstallPlan {
                language: lang.clone(),
                toolchain_version: sec.version.clone(),
                tool_name: name.clone(),
                spec: spec.clone(),
                frozen_carryover,
            });
        }
    }

    // Pruning: tools currently in the lock but no longer in the
    // manifest (skipped under frozen).
    let mut prune_tools = Vec::new();
    if !frozen {
        for (lang, entry) in &lock.backends {
            let manifest_tools: std::collections::HashSet<&str> = manifest
                .get(lang)
                .map(|s| s.tools.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            for t in &entry.tools {
                if !manifest_tools.contains(t.name.as_str()) {
                    if let Ok(language) = LanguageId::new(lang.clone()) {
                        prune_tools.push(ToolPrunePlan {
                            language,
                            tool_name: t.name.clone(),
                        });
                    }
                }
            }
        }
    }

    // Lock header (version + distribution) updates: one per pinned
    // language so the lock's toolchain pins reflect the manifest.
    let lock_header_updates: Vec<LockHeaderUpdate> = manifest
        .iter()
        .map(|(lang, sec)| LockHeaderUpdate {
            language: lang.clone(),
            version: sec.version.clone(),
            distribution: sec.distribution.clone(),
        })
        .collect();

    Ok(SyncPlan {
        install_toolchains,
        install_tools,
        prune_tools,
        lock_header_updates,
        frozen,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use crate::backend::ToolSpec;
    use crate::backends;
    use crate::domain::pinned::validate;
    use crate::lock::{Lock, LockedBackend};
    use crate::manifest::{LanguageSection, Manifest as RawManifest};
    use crate::registry::BackendRegistry;

    fn registry() -> BackendRegistry {
        let mut r = BackendRegistry::new();
        r.register(Arc::new(backends::go::GoBackend));
        r.register(Arc::new(backends::java::JavaBackend));
        r.register(Arc::new(backends::kotlin::KotlinBackend));
        r
    }

    fn raw_with(entries: &[(&str, &str, Option<&str>, &[(&str, &str)])]) -> RawManifest {
        let mut languages: BTreeMap<String, LanguageSection> = BTreeMap::new();
        for (lang, version, distribution, tools) in entries {
            let mut t = BTreeMap::new();
            for (n, v) in *tools {
                t.insert((*n).to_string(), ToolSpec::Short((*v).to_string()));
            }
            languages.insert(
                (*lang).to_string(),
                LanguageSection {
                    version: Some((*version).to_string()),
                    distribution: distribution.map(|d| d.to_string()),
                    tools: t,
                },
            );
        }
        RawManifest { languages }
    }

    #[test]
    fn plan_install_toolchains_emits_one_per_pinned_language() {
        let raw = raw_with(&[
            ("go", "1.26.2", None, &[]),
            ("java", "21", Some("temurin"), &[]),
        ]);
        let pinned = validate(&raw, &registry()).unwrap();
        let plans = plan_install_toolchains(&pinned);
        assert_eq!(plans.len(), 2);
        let go = plans.iter().find(|p| p.language.as_str() == "go").unwrap();
        assert_eq!(go.version.as_str(), "1.26.2");
        assert!(go.distribution.is_none());
        let java = plans.iter().find(|p| p.language.as_str() == "java").unwrap();
        assert_eq!(java.distribution.as_ref().unwrap().as_str(), "temurin");
    }

    #[test]
    fn plan_sync_includes_tools_and_prunes_stale() {
        let raw = raw_with(&[("go", "1.26.2", None, &[("gopls", "latest")])]);
        let pinned = validate(&raw, &registry()).unwrap();

        // Lock has gopls + golangci-lint; manifest only mentions gopls.
        let mut lock = Lock::empty();
        let mut go_entry = LockedBackend::default();
        go_entry.version = "1.26.2".into();
        go_entry.tools = vec![
            crate::backend::LockedTool {
                name: "gopls".into(),
                package: "golang.org/x/tools/gopls".into(),
                version: "v0.21.0".into(),
                bin: "/x/gopls".into(),
                upstream_hash: "h1:abc".into(),
                built_with: "1.26.2".into(),
            },
            crate::backend::LockedTool {
                name: "golangci-lint".into(),
                package: "github.com/golangci/golangci-lint/cmd/golangci-lint".into(),
                version: "v2.0".into(),
                bin: "/x/golangci-lint".into(),
                upstream_hash: "h1:def".into(),
                built_with: "1.26.2".into(),
            },
        ];
        lock.backends.insert("go".into(), go_entry);

        let plan = plan_sync(&pinned, &lock, false).unwrap();
        assert_eq!(plan.install_tools.len(), 1);
        assert_eq!(plan.install_tools[0].tool_name, "gopls");
        assert!(plan.install_tools[0].frozen_carryover.is_none());
        assert_eq!(plan.prune_tools.len(), 1);
        assert_eq!(plan.prune_tools[0].tool_name, "golangci-lint");
    }

    #[test]
    fn plan_sync_frozen_carries_over_lock_entry() {
        let raw = raw_with(&[("go", "1.26.2", None, &[("gopls", "latest")])]);
        let pinned = validate(&raw, &registry()).unwrap();
        let mut lock = Lock::empty();
        let mut go_entry = LockedBackend::default();
        go_entry.version = "1.26.2".into();
        let prev = crate::backend::LockedTool {
            name: "gopls".into(),
            package: "golang.org/x/tools/gopls".into(),
            version: "v0.21.0".into(),
            bin: "/x/gopls".into(),
            upstream_hash: "h1:abc".into(),
            built_with: "1.26.2".into(),
        };
        go_entry.tools = vec![prev.clone()];
        lock.backends.insert("go".into(), go_entry);

        let plan = plan_sync(&pinned, &lock, true).unwrap();
        assert_eq!(plan.install_tools.len(), 1);
        let carry = plan.install_tools[0].frozen_carryover.as_ref().unwrap();
        assert_eq!(carry.version, "v0.21.0");
        // Frozen never prunes.
        assert_eq!(plan.prune_tools.len(), 0);
    }

    #[test]
    fn plan_sync_frozen_errors_when_lock_missing_tool() {
        let raw = raw_with(&[("go", "1.26.2", None, &[("gopls", "latest")])]);
        let pinned = validate(&raw, &registry()).unwrap();
        let lock = Lock::empty();
        let err = plan_sync(&pinned, &lock, true).unwrap_err();
        match err {
            PlanError::FrozenToolMissingFromLock { lang, tool } => {
                assert_eq!(lang, "go");
                assert_eq!(tool, "gopls");
            }
        }
    }
}
