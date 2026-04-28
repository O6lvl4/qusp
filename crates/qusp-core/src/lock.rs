//! `qusp.lock` schema. One section per backend; tools nested inside.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::backend::LockedTool;

pub const LOCK_FILE: &str = "qusp.lock";
pub const LOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lock {
    pub version: u32,
    /// backend id → locked entry.
    #[serde(flatten)]
    pub backends: BTreeMap<String, LockedBackend>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockedBackend {
    pub version: String,
    /// Vendor / distribution selector echoed from the manifest.
    /// Empty/absent when the backend is single-source.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub distribution: String,
    /// Backend-specific verification hash for the toolchain itself.
    /// Empty when not applicable (e.g. ruby compiles from source).
    #[serde(default)]
    pub upstream_hash: String,
    #[serde(default)]
    pub tools: Vec<LockedTool>,
}

impl Lock {
    pub fn empty() -> Self {
        Self {
            version: LOCK_VERSION,
            ..Default::default()
        }
    }

    pub fn load(root: &Path) -> Result<Self> {
        let p = root.join(LOCK_FILE);
        if !p.is_file() {
            return Ok(Self::empty());
        }
        let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", p.display()))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let p = root.join(LOCK_FILE);
        let text =
            toml::to_string_pretty(self).with_context(|| format!("serialize {}", p.display()))?;
        std::fs::write(&p, text).with_context(|| format!("write {}", p.display()))?;
        Ok(())
    }

    pub fn upsert_backend(&mut self, id: &str, entry: LockedBackend) {
        self.backends.insert(id.to_string(), entry);
    }
}
