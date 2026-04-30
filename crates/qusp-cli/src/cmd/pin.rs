use anyhow::{anyhow, bail, Result};
use anyv_core::presentation::{bold as color_bold, cyan as color_cyan, dim, success_mark};
use anyv_core::say;
use clap::Subcommand;
use qusp_core::effects::{FarmManager, GlobalPins};
use qusp_core::registry::BackendRegistry;
use std::process::ExitCode;

use crate::output::OutputFormat;

#[derive(Debug, Subcommand)]
pub enum PinCmd {
    /// Set the global pin for a language. Optional `--distribution`
    /// for multi-vendor backends (java).
    Set {
        lang: String,
        version: String,
        #[arg(long)]
        distribution: Option<String>,
    },
    /// List current global pins.
    List,
    /// Remove the global pin for a language.
    Rm { lang: String },
}

pub async fn cmd_pin(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    cmd: PinCmd,
    _fmt: OutputFormat,
) -> Result<ExitCode> {
    let mut pins = GlobalPins::load(&paths.config).unwrap_or_default();
    match cmd {
        PinCmd::Set {
            lang,
            version,
            distribution,
        } => pin_set(r, paths, &mut pins, &lang, &version, distribution),
        PinCmd::List => pin_list(&pins),
        PinCmd::Rm { lang } => pin_rm(&mut pins, paths, &lang),
    }
}

fn pin_set(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    pins: &mut GlobalPins,
    lang: &str,
    version: &str,
    distribution: Option<String>,
) -> Result<ExitCode> {
    let backend = r
        .get(lang)
        .ok_or_else(|| anyhow!("unknown language: {lang}"))?;
    let installed = backend.list_installed(paths).unwrap_or_default();
    let resolved = installed
        .iter()
        .find(|v| v.as_str() == version)
        .or_else(|| {
            let suffix = format!("-{version}");
            installed.iter().find(|v| v.ends_with(&suffix))
        });
    let resolved = match resolved {
        Some(v) => v.clone(),
        None => bail!(
            "{lang} {version} is not installed via qusp.\n  → run `qusp install {lang} {version}` first, then `qusp pin set {lang} {version}`"
        ),
    };
    let dist = distribution.or_else(|| {
        resolved
            .strip_suffix(&format!("-{version}"))
            .map(|d| d.to_string())
    });
    pins.set(lang, version, dist.as_deref());
    pins.save(&paths.config)?;
    say!(
        "{} pinned {} {} globally",
        success_mark(),
        color_cyan(lang),
        color_bold(version)
    );
    refresh_farm_links(paths, backend.as_ref(), &resolved, version);
    Ok(ExitCode::SUCCESS)
}

fn refresh_farm_links(
    paths: &qusp_core::Paths,
    backend: &dyn qusp_core::backend::Backend,
    resolved: &str,
    version: &str,
) {
    let bins = backend.farm_binaries(version);
    if bins.is_empty() {
        return;
    }
    let install_dir = paths.data.join(backend.id()).join(resolved);
    if !install_dir.exists() {
        return;
    }
    let farm = FarmManager::default();
    let store_root = paths.store();
    if let Ok(r) = farm.install_links(&install_dir, &bins, true, &store_root) {
        if !r.linked.is_empty() {
            say!("  + farm: {}", r.linked.join(", "));
        }
    }
}

fn pin_list(pins: &GlobalPins) -> Result<ExitCode> {
    if pins.pins.is_empty() {
        say!("{}", dim("(no global pins set)"));
        return Ok(ExitCode::SUCCESS);
    }
    for (lang, pin) in &pins.pins {
        let dist = if pin.distribution.is_empty() {
            String::new()
        } else {
            format!(" [{}]", pin.distribution)
        };
        println!(
            "  {} {}{}",
            color_cyan(lang),
            color_bold(&pin.version),
            dim(&dist)
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn pin_rm(pins: &mut GlobalPins, paths: &qusp_core::Paths, lang: &str) -> Result<ExitCode> {
    if pins.remove(lang).is_none() {
        bail!("no global pin set for {lang}");
    }
    pins.save(&paths.config)?;
    say!("{} unpinned {} globally", success_mark(), color_cyan(lang));
    Ok(ExitCode::SUCCESS)
}
