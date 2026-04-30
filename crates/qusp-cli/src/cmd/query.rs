use anyhow::{anyhow, Result};
use anyv_core::presentation::{
    bold as color_bold, cyan as color_cyan, dim, spinner,
};
use anyv_core::say;
use qusp_core::registry::BackendRegistry;
use qusp_core::{lock, manifest};
use std::process::ExitCode;

use crate::output::{self, OutputFormat};

pub fn cmd_backends(r: &BackendRegistry, fmt: OutputFormat) -> Result<ExitCode> {
    let out = output::BackendsOutput {
        backends: r
            .ids()
            .map(|id| output::BackendEntry { id: id.to_string() })
            .collect(),
    };
    fmt.emit(&out);
    Ok(ExitCode::SUCCESS)
}

pub async fn cmd_list(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    lang: &str,
    remote: bool,
    fmt: OutputFormat,
) -> Result<ExitCode> {
    let backend = r
        .get(lang)
        .ok_or_else(|| anyhow!("unknown language: {lang}"))?;
    let (scope, versions) = if remote {
        let client = super::http()?;
        (
            output::ListScope::Remote,
            backend.list_remote(&client).await?,
        )
    } else {
        (output::ListScope::Installed, backend.list_installed(paths)?)
    };
    let out = output::ListOutput {
        lang: lang.to_string(),
        scope,
        versions: versions
            .into_iter()
            .map(|version| output::VersionEntry { version })
            .collect(),
    };
    fmt.emit(&out);
    Ok(ExitCode::SUCCESS)
}

pub async fn cmd_current(
    r: &BackendRegistry,
    lang: Option<&str>,
    fmt: OutputFormat,
) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let langs: Vec<&str> = match lang {
        Some(l) => vec![l],
        None => r.ids().collect(),
    };
    let mut entries = Vec::new();
    for id in langs {
        let backend = match r.get(id) {
            Some(b) => b,
            None => {
                eprintln!("unknown language: {id}");
                continue;
            }
        };
        let entry = match backend.detect_version(&cwd).await? {
            Some(d) => output::CurrentEntry {
                backend: id.to_string(),
                version: Some(d.version),
                source: Some(d.source),
                source_path: Some(output::path_to_string(&d.origin)),
            },
            None => output::CurrentEntry {
                backend: id.to_string(),
                version: None,
                source: None,
                source_path: None,
            },
        };
        entries.push(entry);
    }
    fmt.emit(&output::CurrentOutput { backends: entries });
    Ok(ExitCode::SUCCESS)
}

pub async fn cmd_tree(r: &BackendRegistry, paths: &qusp_core::Paths) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let project_root = manifest::find_root(&cwd);
    println!("{}", color_bold("qusp tree"));
    print_tree_manifest(project_root.as_deref());
    print_tree_resolution(r, &cwd).await?;
    let _ = paths;
    Ok(ExitCode::SUCCESS)
}

fn print_tree_manifest(project_root: Option<&std::path::Path>) {
    let Some(root) = project_root else {
        println!("├── {} {}", color_cyan("manifest"), dim("(no qusp.toml in this tree)"));
        return;
    };
    let m = manifest::load(root).unwrap_or_default();
    let _ = lock::Lock::load(root);
    if m.languages.is_empty() {
        println!("├── {} {}", color_cyan("manifest"), dim("(qusp.toml is empty)"));
        return;
    }
    println!("├── {} {}", color_cyan("manifest"), root.join("qusp.toml").display());
    for (lang, sec) in &m.languages {
        let v = sec.version.as_deref().unwrap_or("(no version)");
        println!("│   ├── {} = {}", lang, color_bold(v));
        for (tname, tspec) in &sec.tools {
            println!("│   │   └── tool {} = \"{}\"", tname, tspec.version());
        }
    }
}

async fn print_tree_resolution(r: &BackendRegistry, cwd: &std::path::Path) -> Result<()> {
    println!("└── {} per-language detection", color_cyan("resolution"));
    let last_id = r.ids().last();
    for (id, backend) in r.iter() {
        let prefix = if Some(id) == last_id { "    └──" } else { "    ├──" };
        match backend.detect_version(cwd).await? {
            Some(d) => println!(
                "{prefix} {} {} {}",
                color_cyan(id),
                color_bold(&d.version),
                dim(&format!("(from {})", d.source))
            ),
            None => println!("{prefix} {} {}", color_cyan(id), dim("(no pin detected)")),
        }
    }
    Ok(())
}

pub async fn cmd_outdated(r: &BackendRegistry, fmt: OutputFormat) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let lock = lock::Lock::load(&root).unwrap_or_else(|_| lock::Lock::empty());
    if lock.backends.is_empty() {
        if matches!(fmt, OutputFormat::Text) {
            say!(
                "{}",
                dim("(qusp.lock has no toolchain entries — run `qusp sync` first)")
            );
        } else {
            fmt.emit(&output::OutdatedOutput { entries: vec![] });
        }
        return Ok(ExitCode::SUCCESS);
    }
    let client = super::http()?;
    let mut entries: Vec<output::OutdatedEntry> = Vec::new();
    for (lang, entry) in &lock.backends {
        let Some(backend) = r.get(lang) else { continue };
        if entry.version.is_empty() {
            continue;
        }
        let pb = if matches!(fmt, OutputFormat::Text) {
            Some(spinner(format!("checking {lang}")))
        } else {
            None
        };
        let remote = backend.list_remote(&client).await;
        if let Some(pb) = pb {
            pb.finish_and_clear();
        }
        if let Some(e) = resolve_outdated_entry(lang, &entry.version, remote) {
            entries.push(e);
        }
    }
    fmt.emit(&output::OutdatedOutput { entries });
    Ok(ExitCode::SUCCESS)
}

fn resolve_outdated_entry(
    lang: &str,
    pinned_raw: &str,
    remote: std::result::Result<Vec<String>, anyhow::Error>,
) -> Option<output::OutdatedEntry> {
    let pinned = pinned_raw.trim().to_string();
    let remote = match remote {
        Ok(r) => r,
        Err(_) => {
            return Some(output::OutdatedEntry {
                backend: lang.to_string(),
                status: output::OutdatedStatus::Unknown,
                current: pinned,
                latest: None,
            });
        }
    };
    let latest_raw = remote.first().cloned().unwrap_or_default();
    let latest = latest_raw
        .split_whitespace()
        .next()
        .unwrap_or(&latest_raw)
        .to_string();
    if latest.is_empty() {
        return None;
    }
    let status = if is_rolling_channel(&pinned) || version_loose_eq(&pinned, &latest) {
        output::OutdatedStatus::UpToDate
    } else {
        output::OutdatedStatus::Outdated
    };
    Some(output::OutdatedEntry {
        backend: lang.to_string(),
        status,
        current: pinned,
        latest: Some(latest),
    })
}

fn version_loose_eq(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> String {
        s.trim()
            .strip_prefix('v')
            .unwrap_or(s)
            .trim_start_matches("go")
            .to_string()
    }
    norm(a) == norm(b)
}

fn is_rolling_channel(pinned: &str) -> bool {
    matches!(
        pinned.trim(),
        "stable" | "beta" | "nightly" | "latest" | "current"
    )
}
