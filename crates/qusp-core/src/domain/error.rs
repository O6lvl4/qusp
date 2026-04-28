//! Typed errors for the domain layer.
//!
//! `anyhow::Error` is fine for the application/CLI layer where every
//! upstream failure flattens into a printable string. Inside the
//! domain we want each failure mode to be a distinct variant so
//! callers can branch on them and so the error messages are uniform
//! across the codebase.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    /// A `[<lang>]` section names a backend that isn't registered.
    #[error("[{lang}] is not a known language. Registered backends: {known}")]
    UnknownLanguage { lang: String, known: String },

    /// A section has no `version` field.
    #[error("[{lang}] is missing a `version` field — add `version = \"...\"`")]
    MissingVersion { lang: String },

    /// `Backend::requires` lists a backend that isn't pinned in the manifest.
    #[error(
        "[{lang}] requires [{required}] to be pinned in qusp.toml — \
         add a [{required}] section with a version before installing {lang}"
    )]
    MissingDependency { lang: String, required: String },
}
