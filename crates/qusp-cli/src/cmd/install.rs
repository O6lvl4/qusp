use anyhow::{anyhow, bail, Result};
use anyv_core::presentation::{
    bold as color_bold, cyan as color_cyan, dim, format_duration_ms, green as color_green,
    spinner, success_mark, yellow as color_yellow,
};
use anyv_core::say;
use qusp_core::registry::BackendRegistry;
use qusp_core::{lock, manifest};
use std::path::Path;
use std::process::ExitCode;

pub async fn cmd_install(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    lang: Option<String>,
    version: Option<String>,
) -> Result<ExitCode> {
    if let (Some(lang), Some(version)) = (lang.as_ref(), version.as_ref()) {
        return install_one(r, paths, lang, version).await;
    }
    if lang.is_some() || version.is_some() {
        bail!("`qusp install` takes either no args (install everything in qusp.toml) or `<lang> <version>`");
    }
    install_all(r, paths).await
}

async fn install_one(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    lang: &str,
    version: &str,
) -> Result<ExitCode> {
    let backend = r
        .get(lang)
        .ok_or_else(|| anyhow!("unknown language: {lang}"))?;
    let cwd = std::env::current_dir()?;
    let project_root = manifest::find_root(&cwd);
    let distribution = project_root
        .as_deref()
        .and_then(|root| manifest::load(root).ok())
        .and_then(|m| m.languages.get(lang).cloned())
        .and_then(|s| s.distribution);
    let opts = qusp_core::InstallOpts {
        distribution: distribution.clone(),
    };
    let http = qusp_core::effects::LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))?;
    let progress = qusp_core::effects::LiveProgress::new();
    let ctx = qusp_core::backend::InstallCtx {
        opts: &opts,
        http: &http,
        progress: &progress,
    };
    let report = backend.install(paths, version, &ctx).await?;
    say!("{} {lang} {} installed", success_mark(), report.version);
    if !report.install_dir.as_os_str().is_empty() {
        say!("  → {}", report.install_dir.display());
    }
    if !report.already_present {
        materialize_farm(paths, lang, backend.as_ref(), &report);
    }
    if let Some(root) = project_root {
        upsert_lock_entry(&root, lang, &report.version, distribution.as_deref());
    }
    Ok(ExitCode::SUCCESS)
}

fn materialize_farm(
    paths: &qusp_core::Paths,
    lang: &str,
    backend: &dyn qusp_core::backend::Backend,
    report: &qusp_core::backend::InstallReport,
) {
    let bins = backend.farm_binaries(&report.version);
    if bins.is_empty() {
        return;
    }
    let global_pins = qusp_core::effects::GlobalPins::load(&paths.config).unwrap_or_default();
    let pin_matches = global_pins
        .get(lang)
        .map(|p| p.version == report.version)
        .unwrap_or(false);
    let farm = qusp_core::effects::FarmManager::default();
    let store_root = paths.store();
    match farm.install_links(&report.install_dir, &bins, pin_matches, &store_root) {
        Ok(r) => {
            if !r.linked.is_empty() {
                say!(
                    "  + farm: {} ({} link{})",
                    r.linked.join(", "),
                    r.linked.len(),
                    if r.linked.len() == 1 { "" } else { "s" }
                );
            }
            for (link, target) in &r.skipped_foreign {
                eprintln!("  {} farm: skipped {link} (held by {target})", color_yellow("!"));
            }
        }
        Err(e) => eprintln!("  {} farm: link install failed: {e:#}", color_yellow("!")),
    }
}

fn upsert_lock_entry(root: &Path, lang: &str, version: &str, distribution: Option<&str>) {
    let mut lock = lock::Lock::load(root).unwrap_or_else(|_| lock::Lock::empty());
    let mut entry = lock.backends.get(lang).cloned().unwrap_or_default();
    entry.version = version.to_string();
    if let Some(d) = distribution {
        entry.distribution = d.to_string();
    }
    lock.upsert_backend(lang, entry);
    if let Err(e) = lock.save(root) {
        eprintln!("{} could not persist qusp.lock: {e:#}", color_yellow("!"));
    }
}

async fn install_all(r: &BackendRegistry, paths: &qusp_core::Paths) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let m = manifest::load(&root)?;
    if m.languages.is_empty() {
        say!("{}", dim("(qusp.toml has no languages pinned)"));
        return Ok(ExitCode::SUCCESS);
    }
    let pinned = qusp_core::domain::validate(&m, r)?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let started = std::time::Instant::now();
    let result = orch.install_toolchains(&pinned).await?;
    let elapsed = started.elapsed().as_millis();
    persist_batch_lock(&root, &result.installed, &m);
    print_install_summary(&result, elapsed)
}

fn persist_batch_lock(
    root: &Path,
    installed: &[qusp_core::orchestrator::InstallSummary],
    m: &qusp_core::manifest::Manifest,
) {
    if installed.is_empty() {
        return;
    }
    let mut lock = lock::Lock::load(root).unwrap_or_else(|_| lock::Lock::empty());
    for s in installed {
        let mut entry = lock.backends.get(&s.lang).cloned().unwrap_or_default();
        entry.version = s.version.clone();
        if let Some(sec) = m.languages.get(&s.lang) {
            if let Some(d) = &sec.distribution {
                entry.distribution = d.clone();
            }
        }
        lock.upsert_backend(&s.lang, entry);
    }
    if let Err(e) = lock.save(root) {
        eprintln!("{} could not persist qusp.lock: {e:#}", color_yellow("!"));
    }
}

fn print_install_summary(
    result: &qusp_core::orchestrator::InstallToolchainsResult,
    elapsed: u128,
) -> Result<ExitCode> {
    say!(
        "{} Installed {} toolchain{} in {}",
        success_mark(),
        result.installed.len(),
        if result.installed.len() == 1 { "" } else { "s" },
        format_duration_ms(elapsed)
    );
    for s in &result.installed {
        let (mark, note) = if s.already_present {
            (dim("="), dim("(already present)"))
        } else {
            (color_green("+"), dim("(installed)"))
        };
        println!(" {mark} {} {} {note}", color_cyan(&s.lang), color_bold(&s.version));
    }
    if !result.failed.is_empty() {
        eprintln!();
        eprintln!(
            "{} {} toolchain{} failed:",
            color_yellow("!"),
            result.failed.len(),
            if result.failed.len() == 1 { "" } else { "s" }
        );
        for (lang, err) in &result.failed {
            eprintln!("  {} {}: {}", color_yellow("✗"), color_cyan(lang), err);
        }
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

pub async fn cmd_sync(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    frozen: bool,
) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let m = manifest::load(&root)?;
    let pinned = qusp_core::domain::validate(&m, r)?;
    let mut lock = lock::Lock::load(&root)?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let client = super::http()?;
    let started = std::time::Instant::now();
    let summary = orch.sync(&pinned, &mut lock, frozen, &client).await?;
    let elapsed = started.elapsed().as_millis();
    print_sync_summary(&summary, elapsed);
    if !frozen {
        lock.save(&root)?;
        say!("{} wrote {}", success_mark(), root.join("qusp.lock").display());
    }
    Ok(ExitCode::SUCCESS)
}

fn print_sync_summary(summary: &qusp_core::orchestrator::SyncSummary, elapsed: u128) {
    say!(
        "{} Synced {} toolchain{} + {} tool{} in {}",
        success_mark(),
        summary.langs_installed.len(),
        if summary.langs_installed.len() == 1 { "" } else { "s" },
        summary.tools_installed.len(),
        if summary.tools_installed.len() == 1 { "" } else { "s" },
        format_duration_ms(elapsed)
    );
    for s in &summary.langs_installed {
        let mark = if s.already_present { dim("=") } else { color_green("+") };
        println!(" {mark} {} {}", color_cyan(&s.lang), color_bold(&s.version));
    }
    for (lang, locked) in &summary.tools_installed {
        println!(
            " {} {}/{} {}",
            color_green("+"),
            color_cyan(lang),
            color_bold(&locked.name),
            dim(&locked.version)
        );
    }
    if summary.tools_removed_from_lock > 0 {
        println!(
            " {} pruned {} stale lock entr{}",
            dim("-"),
            summary.tools_removed_from_lock,
            if summary.tools_removed_from_lock == 1 { "y" } else { "ies" }
        );
    }
    if !summary.langs_failed.is_empty() {
        eprintln!();
        eprintln!(
            "{} {} toolchain{} failed (other backends still installed):",
            color_yellow("!"),
            summary.langs_failed.len(),
            if summary.langs_failed.len() == 1 { "" } else { "s" }
        );
        for (lang, err) in &summary.langs_failed {
            eprintln!("  {} {}: {}", color_yellow("✗"), color_cyan(lang), err);
        }
    }
}

pub async fn cmd_add_tool(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    spec: &str,
) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd).ok_or_else(|| {
        anyhow!(
            "no qusp.toml found above {}; run `qusp install <lang> <version>` and create one first",
            cwd.display()
        )
    })?;
    let mut m = manifest::load(&root)?;
    let mut lock = lock::Lock::load(&root)?;
    let (name, version) = match spec.rsplit_once('@') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (spec.to_string(), "latest".to_string()),
    };
    let client = super::http()?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let pb = spinner(format!("resolving {name}"));
    let (lang, locked) = orch
        .add_tool(&mut m, &mut lock, &name, &version, &client)
        .await?;
    pb.finish_and_clear();
    manifest::save(&root, &m)?;
    lock.save(&root)?;
    println!(
        "{} routed to {} backend → {}@{}",
        success_mark(),
        color_cyan(&lang),
        color_bold(&locked.name),
        locked.version
    );
    Ok(ExitCode::SUCCESS)
}
