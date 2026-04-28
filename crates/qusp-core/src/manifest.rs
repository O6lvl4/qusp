//! `qusp.toml` parsing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::backend::ToolSpec;

pub const MANIFEST_FILE: &str = "qusp.toml";

/// Top-level `qusp.toml`. Each `[<lang>]` section becomes a value here
/// keyed by the backend id ("go", "ruby", …).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    /// Per-language pin: `[go] version = "1.26.2"` etc.
    /// We deserialize into a flat map first; backends interpret the inner
    /// section however they like.
    #[serde(flatten)]
    pub languages: BTreeMap<String, LanguageSection>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LanguageSection {
    #[serde(default)]
    pub version: Option<String>,
    /// Vendor / distribution selector. Used by backends like Java where
    /// the same version number resolves differently per vendor (Temurin
    /// vs Corretto vs GraalVM). Single-source backends (Go, Node, …)
    /// ignore this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution: Option<String>,
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSpec>,
}

pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut dir: Option<&Path> = Some(start);
    while let Some(d) = dir {
        if d.join(MANIFEST_FILE).is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

pub fn load(root: &Path) -> Result<Manifest> {
    let p = root.join(MANIFEST_FILE);
    if !p.is_file() {
        return Ok(Manifest::default());
    }
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", p.display()))
}

pub fn save(root: &Path, m: &Manifest) -> Result<()> {
    let p = root.join(MANIFEST_FILE);
    let text = toml::to_string_pretty(m).with_context(|| format!("serialize {}", p.display()))?;
    std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))?;
    Ok(())
}
