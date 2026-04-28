//! Domain types for qusp.
//!
//! Phase 1 of the Functional-DDD-style migration. The aim: lift validation
//! out of `Orchestrator::*` and out of CLI handlers, and into smart-
//! constructed types whose existence is the proof of validity.
//!
//! Boundaries:
//! - [`raw::Manifest`] (re-export of the serde-direct `crate::manifest::Manifest`)
//!   represents un-trusted input — what's on disk.
//! - [`pinned::PinnedManifest`] is the validated form. Construction goes
//!   through [`validate`], which:
//!   - Rejects unknown languages.
//!   - Requires every section to have a `version`.
//!   - Enforces cross-backend dependencies declared via `Backend::requires`.
//!
//! Once you hold a `&PinnedManifest`, the orchestrator can install /
//! sync without re-checking these invariants.
//!
//! Future phases will:
//! - Split toolchain-version → `Version` newtype.
//! - Pull plan generation (`plan_installs(...) -> Vec<InstallPlan>`)
//!   out of the orchestrator into pure functions.
//! - Move the `Backend` trait's IO operations behind explicit effect
//!   traits (HttpFetcher, Filesystem, Extractor) so backends can be
//!   tested without the network.

pub mod error;
pub mod pinned;

pub use error::ManifestError;
pub use pinned::{validate, PinnedManifest, PinnedSection};
