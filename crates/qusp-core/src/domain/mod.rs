//! Domain types for qusp.
//!
//! Functional-DDD-style migration. The aim: lift validation and plan
//! generation out of `Orchestrator::*` and the CLI handlers, into
//! pure types and pure functions. Effects (HTTP, FS, archive
//! extraction) stay in the orchestrator's `execute_*` methods.
//!
//! Layering, top-down:
//!
//! - **types** — newtypes (`LanguageId`, `Version`, `Distribution`).
//!   Smart constructors validate at the boundary; downstream code
//!   never sees a String where a Version belongs.
//! - **error** — typed errors per layer (`ManifestError`, `PlanError`,
//!   the umbrella `DomainError`).
//! - **pinned** — `PinnedManifest` smart-constructed from a serde
//!   `Manifest`. Cross-backend `requires(...)` checks happen here.
//! - **plan** — pure functions that turn a `PinnedManifest` (+ a
//!   `Lock` when relevant) into `InstallPlan` / `SyncPlan`.
//!   No IO. Trivially unit-testable.
//!
//! The orchestrator's `execute_*` methods take these plans and run
//! them. Existing high-level `install_toolchains` / `sync` methods
//! stay as wrappers that compose `plan_*` + `execute_*`, so callers
//! upstream don't change.

pub mod error;
pub mod pinned;
pub mod plan;
pub mod types;

pub use error::{DomainError, ManifestError, PlanError};
pub use pinned::{validate, PinnedManifest, PinnedSection};
pub use plan::{
    plan_install_toolchains, plan_sync, InstallPlan, LockHeaderUpdate, SyncPlan, ToolInstallPlan,
    ToolPrunePlan,
};
pub use types::{Distribution, LanguageId, Version};
