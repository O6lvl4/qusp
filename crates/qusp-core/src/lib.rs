//! qusp-core — the multi-language orchestration core.
//!
//! Wraps [`anyv-core`](https://github.com/O6lvl4/anyv-core) with the
//! qusp-specific bits: the per-language `Backend` trait, the `qusp.toml`
//! manifest schema, the `qusp.lock` lockfile schema, and a small
//! orchestrator that fans out across registered backends.

pub mod backend;
pub mod backends;
pub mod lock;
pub mod manifest;
pub mod orchestrator;
pub mod paths;
pub mod registry;

pub use anyv_core::Paths;
pub use backend::{
    Backend, DetectedVersion, InstallReport, LockedTool, ResolvedTool, RunEnv, ToolSpec,
};
pub use registry::BackendRegistry;
