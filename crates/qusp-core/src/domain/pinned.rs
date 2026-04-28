//! Validated manifest type. Constructed only by [`validate`].
//!
//! Holding a `&PinnedManifest` is a compile-time proof that:
//! - every entry's language id is registered with the backend registry,
//! - every entry has a non-empty `version`,
//! - cross-backend `requires(...)` dependencies are satisfied.
//!
//! The orchestrator's install/sync flow takes `&PinnedManifest` instead
//! of `&Manifest`, so the "did we already validate?" question vanishes.

use std::collections::BTreeMap;

use crate::backend::ToolSpec;
use crate::domain::error::ManifestError;
use crate::manifest::Manifest as RawManifest;
use crate::registry::BackendRegistry;

/// Public surface mirrors the raw section fields, but `version` is
/// guaranteed non-empty by the `validate` smart constructor.
#[derive(Debug, Clone)]
pub struct PinnedSection {
    pub version: String,
    pub distribution: Option<String>,
    pub tools: BTreeMap<String, ToolSpec>,
}

#[derive(Debug, Clone)]
pub struct PinnedManifest {
    entries: BTreeMap<String, PinnedSection>,
}

impl PinnedManifest {
    /// Iterate `(lang_id, section)` in BTreeMap (alphabetical) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &PinnedSection)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Look up a section by language id.
    pub fn get(&self, lang: &str) -> Option<&PinnedSection> {
        self.entries.get(lang)
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
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|k| k.as_str())
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
    let mut entries: BTreeMap<String, PinnedSection> = BTreeMap::new();
    for (lang, sec) in &raw.languages {
        // Unknown language → reject, with the registered set in the message.
        if registry.get(lang).is_none() {
            return Err(ManifestError::UnknownLanguage {
                lang: lang.clone(),
                known: registry.ids().collect::<Vec<_>>().join(", "),
            });
        }
        let version = sec
            .version
            .clone()
            .ok_or_else(|| ManifestError::MissingVersion { lang: lang.clone() })?;
        if version.trim().is_empty() {
            return Err(ManifestError::MissingVersion { lang: lang.clone() });
        }
        entries.insert(
            lang.clone(),
            PinnedSection {
                version,
                distribution: sec.distribution.clone(),
                tools: sec.tools.clone(),
            },
        );
    }

    // Cross-backend deps: every `requires(...)` must be in the manifest.
    for (lang, _) in entries.iter() {
        let backend = registry.get(lang).expect("pre-checked above");
        for required in backend.requires() {
            if !entries.contains_key(*required) {
                return Err(ManifestError::MissingDependency {
                    lang: lang.clone(),
                    required: (*required).to_string(),
                });
            }
        }
    }

    Ok(PinnedManifest { entries })
}
