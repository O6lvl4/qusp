//! Validated manifest type. Constructed only by [`validate`].
//!
//! Holding a `&PinnedManifest` is a compile-time proof that:
//! - every entry's language id is registered with the backend registry,
//! - every entry has a non-empty `version` (held as a [`Version`] newtype),
//! - cross-backend `requires(...)` dependencies are satisfied.
//!
//! The orchestrator's install/sync flow takes `&PinnedManifest` instead
//! of `&Manifest`, so the "did we already validate?" question vanishes.

use std::collections::BTreeMap;

use crate::backend::ToolSpec;
use crate::domain::error::ManifestError;
use crate::domain::types::{Distribution, LanguageId, Version};
use crate::manifest::Manifest as RawManifest;
use crate::registry::BackendRegistry;

/// Public surface mirrors the raw section fields, but the version is
/// non-empty by construction (it's a [`Version`] newtype).
#[derive(Debug, Clone)]
pub struct PinnedSection {
    pub version: Version,
    pub distribution: Option<Distribution>,
    pub tools: BTreeMap<String, ToolSpec>,
}

#[derive(Debug, Clone)]
pub struct PinnedManifest {
    entries: BTreeMap<LanguageId, PinnedSection>,
}

impl PinnedManifest {
    /// Iterate `(LanguageId, section)` in BTreeMap (alphabetical) order.
    pub fn iter(&self) -> impl Iterator<Item = (&LanguageId, &PinnedSection)> {
        self.entries.iter()
    }

    /// Look up a section by language id (str-keyed for ergonomics).
    pub fn get(&self, lang: &str) -> Option<&PinnedSection> {
        self.entries
            .iter()
            .find(|(k, _)| k.as_str() == lang)
            .map(|(_, v)| v)
    }

    /// True when no languages are pinned.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of pinned languages.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Backend ids in alphabetical order.
    pub fn languages(&self) -> impl Iterator<Item = &LanguageId> {
        self.entries.keys()
    }
}

/// Smart constructor: turn a serde-direct `Manifest` into a validated
/// `PinnedManifest`. Returns the **first** error encountered. The errors
/// are typed (see [`ManifestError`]) so callers can branch on them; the
/// CLI just `Display`s.
pub fn validate(
    raw: &RawManifest,
    registry: &BackendRegistry,
) -> Result<PinnedManifest, ManifestError> {
    let mut entries: BTreeMap<LanguageId, PinnedSection> = BTreeMap::new();
    for (lang_str, sec) in &raw.languages {
        // Unknown language → reject, with the registered set in the message.
        if registry.get(lang_str).is_none() {
            return Err(ManifestError::UnknownLanguage {
                lang: lang_str.clone(),
                known: registry.ids().collect::<Vec<_>>().join(", "),
            });
        }
        let raw_version = sec
            .version
            .clone()
            .ok_or_else(|| ManifestError::MissingVersion {
                lang: lang_str.clone(),
            })?;
        let version = Version::new(&raw_version).map_err(|_| ManifestError::EmptyVersion {
            lang: lang_str.clone(),
        })?;
        let distribution = match &sec.distribution {
            Some(d) => Some(
                Distribution::new(d).map_err(|_| ManifestError::EmptyVersion {
                    lang: lang_str.clone(),
                })?,
            ),
            None => None,
        };
        // LanguageId is guaranteed valid because the registry rejects
        // anything we wouldn't accept; using expect here documents that.
        let lang = LanguageId::new(lang_str).expect(
            "registry registration enforces lowercase-ascii ids — this should be unreachable",
        );
        entries.insert(
            lang,
            PinnedSection {
                version,
                distribution,
                tools: sec.tools.clone(),
            },
        );
    }

    // Cross-backend deps: every `requires(...)` must be in the manifest.
    for lang in entries.keys() {
        let backend = registry.get(lang.as_str()).expect("pre-checked above");
        for required in backend.requires() {
            if !entries.keys().any(|k| k.as_str() == *required) {
                return Err(ManifestError::MissingDependency {
                    lang: lang.as_str().to_string(),
                    required: (*required).to_string(),
                });
            }
        }
    }

    Ok(PinnedManifest { entries })
}
