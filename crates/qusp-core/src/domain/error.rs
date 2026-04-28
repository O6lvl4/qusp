//! Typed errors for the domain layer.
//!
//! `anyhow::Error` is fine for the application/CLI layer where every
//! upstream failure flattens into a printable string. Inside the
//! domain we want each failure mode to be a distinct variant so
//! callers can branch on them and so the error messages are uniform
//! across the codebase.

use thiserror::Error;

/// Any failure originating in the domain layer (newtypes, validate,
/// plan generation). Application-layer errors (network, IO, archive
/// extraction) live in `anyhow::Error` and converge at the CLI.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("manifest: {0}")]
    Manifest(#[from] ManifestError),

    #[error("'{0}' is not a valid language id (lowercase ASCII letters/digits/underscore only)")]
    InvalidLanguageId(String),

    #[error("version cannot be empty after trimming whitespace")]
    InvalidVersion,

    #[error("distribution cannot be empty after trimming whitespace")]
    InvalidDistribution,

    #[error("plan: {0}")]
    Plan(#[from] PlanError),
}

#[derive(Debug, Error)]
pub enum ManifestError {
    /// A `[<lang>]` section names a backend that isn't registered.
    #[error("[{lang}] is not a known language. Registered backends: {known}")]
    UnknownLanguage { lang: String, known: String },

    /// A section has no `version` field.
    #[error("[{lang}] is missing a `version` field — add `version = \"...\"`")]
    MissingVersion { lang: String },

    /// A section's `version` is empty after trimming.
    #[error("[{lang}] has an empty `version` — add a non-empty value")]
    EmptyVersion { lang: String },

    /// `Backend::requires` lists a backend that isn't pinned in the manifest.
    #[error(
        "[{lang}] requires [{required}] to be pinned in qusp.toml — \
         add a [{required}] section with a version before installing {lang}"
    )]
    MissingDependency { lang: String, required: String },
}

/// Errors from `plan_*` functions. Plan generation is pure but can
/// still detect inconsistencies the validate layer didn't catch (e.g.
/// `--frozen` sync against a tool that's missing from the lock).
#[derive(Debug, Error)]
pub enum PlanError {
    #[error(
        "frozen sync: {lang} tool '{tool}' is in qusp.toml but not in qusp.lock — \
         drop --frozen and re-run, or commit a fresh lock"
    )]
    FrozenToolMissingFromLock { lang: String, tool: String },
}
