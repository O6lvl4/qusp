//! Domain newtypes.
//!
//! These are lightweight wrappers that turn String fields with implicit
//! invariants ("non-empty version", "lowercase ASCII language id") into
//! types whose construction proves the invariant. They intentionally do
//! **not** parse the underlying string further — versions across our
//! eight publishers don't agree on a format (`go1.26.2`, `21.0.11+10`,
//! `stable`, `temurin/21.0.5`), so we avoid baking semver assumptions
//! at this layer.

use std::fmt;

use crate::domain::error::DomainError;

/// A backend / language id (`"go"`, `"java"`, `"kotlin"`, …).
///
/// Constraints: non-empty, ASCII lowercase + digits + underscores. The
/// registry's stable ids satisfy this; this newtype guards against
/// accidentally building a manifest entry with `"Go"` or `"node-js"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LanguageId(String);

impl LanguageId {
    pub fn new(s: impl Into<String>) -> Result<Self, DomainError> {
        let s = s.into();
        if s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(DomainError::InvalidLanguageId(s));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for LanguageId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A toolchain version pin. Trimmed, non-empty, otherwise opaque.
///
/// Backend-specific normalization (e.g. Go's `1.26.2` → `go1.26.2`)
/// stays in the backend — this type just guarantees "the user wrote
/// something".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version(String);

impl Version {
    pub fn new(s: impl Into<String>) -> Result<Self, DomainError> {
        let s = s.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidVersion);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Version {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Vendor selector for backends with multiple implementations
/// (`temurin`, `corretto`, `zulu`, `graalvm_community` for Java).
/// Single-source backends never construct one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Distribution(String);

impl Distribution {
    pub fn new(s: impl Into<String>) -> Result<Self, DomainError> {
        let s = s.into();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidDistribution);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Distribution {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
