//! Output format dispatch — text (human-readable, colored) vs JSON.
//!
//! Phase 5 (Hospitality Parity), audit row R1.
//!
//! Each introspection-style subcommand (`backends`, `list`, `current`,
//! `doctor`, `dir`, `outdated`, `tree`) builds a typed output struct,
//! then dispatches to either:
//!
//! - **Text**: a hand-rolled colored renderer (the existing pre-R1 output)
//! - **Json**: `serde_json::to_string_pretty` over the same struct
//!
//! The two paths share the data model so they can never drift out of
//! sync — change a field in the struct and both renderers see it.
//!
//! ## Stability
//!
//! The JSON schema is part of qusp's stability contract. Additive
//! changes (new fields) are allowed in minor versions; renames /
//! removals require a major bump. Schema reference lives at
//! `docs/JSON_SCHEMA.md`.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn emit<T: Renderable>(self, item: &T) {
        match self {
            OutputFormat::Text => item.render_text(),
            OutputFormat::Json => {
                let s = serde_json::to_string_pretty(item)
                    .expect("serialize output struct (qusp internal types are infallible)");
                println!("{}", s);
            }
        }
    }
}

/// Marker trait for output types: knows how to render itself for both
/// text and JSON. The `Serialize` bound covers JSON; `render_text` is
/// the human-facing renderer.
pub trait Renderable: Serialize {
    fn render_text(&self);
}

// ─── `qusp backends` ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BackendsOutput {
    pub backends: Vec<BackendEntry>,
}

#[derive(Debug, Serialize)]
pub struct BackendEntry {
    pub id: String,
}

impl Renderable for BackendsOutput {
    fn render_text(&self) {
        use anyv_core::presentation::{bold, cyan};
        println!("{}", bold("qusp backends"));
        for b in &self.backends {
            println!("  {}", cyan(&b.id));
        }
    }
}

// ─── `qusp list <lang>` ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ListOutput {
    pub lang: String,
    pub scope: ListScope,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListScope {
    Installed,
    Remote,
}

#[derive(Debug, Serialize)]
pub struct VersionEntry {
    pub version: String,
}

impl Renderable for ListOutput {
    fn render_text(&self) {
        for v in &self.versions {
            println!("{}", v.version);
        }
    }
}

// ─── `qusp current [lang]` ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CurrentOutput {
    pub backends: Vec<CurrentEntry>,
}

#[derive(Debug, Serialize)]
pub struct CurrentEntry {
    pub backend: String,
    /// `None` means no version is pinned for this backend in cwd.
    pub version: Option<String>,
    /// File or convention that pinned it (e.g. `.python-version`).
    pub source: Option<String>,
    /// Absolute path to the file that pinned it (None for synthetic
    /// sources or no pin).
    pub source_path: Option<String>,
}

impl Renderable for CurrentOutput {
    fn render_text(&self) {
        use anyv_core::presentation::{bold, cyan, dim};
        for entry in &self.backends {
            match (&entry.version, &entry.source) {
                (Some(v), Some(s)) => println!(
                    "{:<10} {} {}",
                    cyan(&entry.backend),
                    bold(v),
                    dim(&format!("(from {s})"))
                ),
                _ => println!("{:<10} {}", cyan(&entry.backend), dim("(none)")),
            }
        }
    }
}

// ─── `qusp doctor` ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DoctorOutput {
    pub qusp_version: String,
    pub paths: DoctorPaths,
    pub backends: Vec<DoctorBackend>,
}

#[derive(Debug, Serialize)]
pub struct DoctorPaths {
    pub data: String,
    pub config: String,
    pub cache: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorBackend {
    pub id: String,
    pub installed_count: usize,
}

impl Renderable for DoctorOutput {
    fn render_text(&self) {
        use anyv_core::presentation::{bold, cyan, green, yellow};
        println!("{}", bold("qusp doctor"));
        println!("  qusp       : {}", &self.qusp_version);
        println!("  data dir   : {}", &self.paths.data);
        println!("  config dir : {}", &self.paths.config);
        println!("  cache dir  : {}", &self.paths.cache);
        let ids: Vec<&str> = self.backends.iter().map(|b| b.id.as_str()).collect();
        println!("  backends   : {}", ids.join(", "));
        for b in &self.backends {
            let mark = if b.installed_count > 0 {
                green(&format!("{} installed", b.installed_count)).to_string()
            } else {
                yellow("none installed yet").to_string()
            };
            println!("  {:<10} : {mark}", cyan(&b.id));
        }
    }
}

// ─── `qusp dir <kind>` ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DirOutput {
    pub kind: String,
    pub path: String,
}

impl Renderable for DirOutput {
    fn render_text(&self) {
        // Match historical bare-path output for `qusp dir <kind>` so
        // shell users like `cd "$(qusp dir data)"` keep working.
        println!("{}", self.path);
    }
}

// ─── `qusp outdated` ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OutdatedOutput {
    pub entries: Vec<OutdatedEntry>,
}

#[derive(Debug, Serialize)]
pub struct OutdatedEntry {
    pub backend: String,
    pub status: OutdatedStatus,
    pub current: String,
    /// Latest known upstream version. Absent if upstream lookup failed.
    pub latest: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutdatedStatus {
    /// Current pin matches latest upstream.
    UpToDate,
    /// Upstream has a newer version than current pin.
    Outdated,
    /// Upstream lookup failed.
    Unknown,
}

impl Renderable for OutdatedOutput {
    fn render_text(&self) {
        use anyv_core::presentation::{bold, cyan, dim, green, success_mark, yellow};
        let mut hits = 0usize;
        for e in &self.entries {
            match e.status {
                OutdatedStatus::UpToDate => println!(
                    " {} {} {}",
                    green("="),
                    cyan(&e.backend),
                    bold(&e.current)
                ),
                OutdatedStatus::Outdated => {
                    hits += 1;
                    let latest_disp = e.latest.as_deref().unwrap_or("?");
                    println!(
                        " {} {} {} → {}",
                        yellow("↑"),
                        cyan(&e.backend),
                        bold(&e.current),
                        bold(latest_disp)
                    );
                }
                OutdatedStatus::Unknown => println!(
                    " {} {}: {}",
                    yellow("?"),
                    cyan(&e.backend),
                    dim("could not query upstream")
                ),
            }
        }
        if hits == 0 {
            println!("\n{} all toolchains at latest", success_mark());
        } else {
            let (noun, verb) = if hits == 1 {
                ("toolchain", "has")
            } else {
                ("toolchains", "have")
            };
            println!(
                "\n{} {hits} {noun} {verb} newer upstream versions. \
                 Bump qusp.toml and run `qusp sync` to apply.",
                yellow("!"),
            );
        }
    }
}

// ─── helpers ────────────────────────────────────────────────────────

pub fn path_to_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backends_json_round_trip() {
        let o = BackendsOutput {
            backends: vec![
                BackendEntry { id: "python".into() },
                BackendEntry { id: "ruby".into() },
            ],
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"id\":\"python\""));
        assert!(json.contains("\"id\":\"ruby\""));
    }

    #[test]
    fn current_serializes_none_as_null() {
        let o = CurrentOutput {
            backends: vec![CurrentEntry {
                backend: "ruby".into(),
                version: None,
                source: None,
                source_path: None,
            }],
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"version\":null"));
        assert!(json.contains("\"source\":null"));
    }

    #[test]
    fn list_scope_serializes_snake_case() {
        let o = ListOutput {
            lang: "lua".into(),
            scope: ListScope::Installed,
            versions: vec![VersionEntry { version: "5.4.7".into() }],
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(
            json.contains("\"scope\":\"installed\""),
            "scope should serialize lowercase: {json}"
        );
    }

    #[test]
    fn doctor_json_has_all_fields() {
        let o = DoctorOutput {
            qusp_version: "0.25.0".into(),
            paths: DoctorPaths {
                data: "/tmp/data".into(),
                config: "/tmp/config".into(),
                cache: "/tmp/cache".into(),
            },
            backends: vec![DoctorBackend {
                id: "python".into(),
                installed_count: 2,
            }],
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"qusp_version\":\"0.25.0\""));
        assert!(json.contains("\"installed_count\":2"));
    }
}
