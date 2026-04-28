//! qusp CLI — v0.2.0.
//!
//! Native Go/Ruby/Python backends + the orchestrator: `qusp install`
//! (no args = parallel install of every language pinned in qusp.toml),
//! `qusp sync [--frozen]`, `qusp add tool` (auto-routed via each
//! backend's static registry), `qusp run` (merges PATH + GOROOT +
//! GEM_HOME + … across backends), `quspx` (ephemeral run via argv[0]
//! dispatch).

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use anyv_core::presentation::{
    bold as color_bold, cyan as color_cyan, dim, format_duration_ms, green as color_green,
    set_quiet, spinner, success_mark, yellow as color_yellow,
};
use anyv_core::say;
use clap::{Parser, Subcommand};
use qusp_core::backends;
use qusp_core::registry::BackendRegistry;
use qusp_core::{lock, manifest, paths};

#[derive(Debug, Parser)]
#[command(
    name = "qusp",
    version,
    about = "Every language toolchain in superposition. `cd` collapses to one.",
    propagate_version = true
)]
struct Cli {
    #[arg(short = 'q', long = "quiet", global = true)]
    quiet: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// List the languages this build of qusp supports.
    Backends,
    /// Install toolchains. With no args, installs everything pinned in
    /// qusp.toml (in parallel). With `<lang> <version>`, installs just one.
    Install {
        lang: Option<String>,
        version: Option<String>,
    },
    /// Reconcile installs with qusp.toml + qusp.lock. With --frozen,
    /// refuses to update qusp.lock and uses lock-as-truth.
    Sync {
        #[arg(long)]
        frozen: bool,
    },
    /// Pin and install a tool. Auto-routes to the backend whose registry
    /// recognizes the name.
    Add {
        #[command(subcommand)]
        target: AddCmd,
    },
    /// Run a command using the resolved multi-language environment.
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        argv: Vec<String>,
    },
    /// Ephemeral run. argv[0]=`quspx` dispatches into this. Resolves, installs
    /// (or reuses), executes — without touching qusp.toml/qusp.lock.
    X {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        argv: Vec<String>,
    },
    /// List installed toolchains, or remote ones with --remote.
    List {
        lang: String,
        #[arg(long)]
        remote: bool,
    },
    /// Resolve and report which version applies in the current directory.
    Current { lang: Option<String> },
    /// Show the resolved multi-language environment.
    Tree,
    /// Print a gv/uv-style health check.
    Doctor,
    /// Print path of a gv-managed dir (data, cache, …).
    Dir {
        #[arg(value_enum)]
        kind: DirKind,
    },
    /// Generate shell completions.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
enum AddCmd {
    /// `qusp add tool gopls` — auto-detected as a Go tool.
    Tool { spec: String },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum DirKind {
    Data,
    Cache,
    Config,
}

fn main() -> ExitCode {
    let cli = match anyv_core::argv0::rewrite_for_x_dispatch("qusp") {
        Some(rewritten) => Cli::parse_from(rewritten),
        None => Cli::parse(),
    };
    set_quiet(cli.quiet);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    match rt.block_on(run(cli)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    let paths = paths::discover()?;
    paths.ensure_dirs()?;
    let registry = build_registry();

    match cli.cmd {
        Cmd::Backends => cmd_backends(&registry),
        Cmd::Install { lang, version } => cmd_install(&registry, &paths, lang, version).await,
        Cmd::Sync { frozen } => cmd_sync(&registry, &paths, frozen).await,
        Cmd::Add { target } => match target {
            AddCmd::Tool { spec } => cmd_add_tool(&registry, &paths, &spec).await,
        },
        Cmd::Run { argv } => cmd_run(&registry, &paths, argv),
        Cmd::X { argv } => cmd_x(&registry, &paths, argv).await,
        Cmd::List { lang, remote } => cmd_list(&registry, &paths, &lang, remote).await,
        Cmd::Current { lang } => cmd_current(&registry, lang.as_deref()).await,
        Cmd::Tree => cmd_tree(&registry, &paths).await,
        Cmd::Doctor => cmd_doctor(&registry, &paths),
        Cmd::Dir { kind } => cmd_dir(&paths, kind),
        Cmd::Completions { shell } => cmd_completions(shell),
    }
}

fn build_registry() -> BackendRegistry {
    let mut r = BackendRegistry::new();
    r.register(Arc::new(backends::go::GoBackend));
    r.register(Arc::new(backends::ruby::RubyBackend));
    r.register(Arc::new(backends::python::PythonBackend));
    r
}

fn cmd_backends(r: &BackendRegistry) -> Result<ExitCode> {
    println!("{}", color_bold("qusp backends"));
    for id in r.ids() {
        println!("  {}", color_cyan(id));
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_install(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    lang: Option<String>,
    version: Option<String>,
) -> Result<ExitCode> {
    if let (Some(lang), Some(version)) = (lang.as_ref(), version.as_ref()) {
        let backend = r
            .get(lang)
            .ok_or_else(|| anyhow!("unknown language: {lang}"))?;
        let pb = spinner(format!("installing {lang} {version}"));
        let report = backend.install(paths, version).await?;
        pb.finish_and_clear();
        say!("{} {lang} {} installed", success_mark(), report.version);
        if !report.install_dir.as_os_str().is_empty() {
            say!("  → {}", report.install_dir.display());
        }
        return Ok(ExitCode::SUCCESS);
    }
    if lang.is_some() || version.is_some() {
        bail!("`qusp install` takes either no args (install everything in qusp.toml) or `<lang> <version>`");
    }
    // No args → install everything in the manifest, parallel across backends.
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let m = manifest::load(&root)?;
    if m.languages.is_empty() {
        say!("{}", dim("(qusp.toml has no languages pinned)"));
        return Ok(ExitCode::SUCCESS);
    }
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let started = std::time::Instant::now();
    let summaries = orch.install_toolchains(&m).await?;
    let elapsed = started.elapsed().as_millis();
    say!(
        "{} Installed {} toolchain{} in {}",
        success_mark(),
        summaries.len(),
        if summaries.len() == 1 { "" } else { "s" },
        format_duration_ms(elapsed)
    );
    for s in &summaries {
        let mark = if s.already_present { dim("=") } else { color_green("+") };
        let note = if s.already_present { dim("(already present)") } else { dim("(installed)") };
        println!(" {mark} {} {} {note}", color_cyan(&s.lang), color_bold(&s.version));
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_sync(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    frozen: bool,
) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let m = manifest::load(&root)?;
    let mut lock = lock::Lock::load(&root)?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let client = http_client()?;
    let started = std::time::Instant::now();
    let summary = orch.sync(&m, &mut lock, frozen, &client).await?;
    let elapsed = started.elapsed().as_millis();
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
    if !frozen {
        lock.save(&root)?;
        say!("{} wrote {}", success_mark(), root.join("qusp.lock").display());
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_add_tool(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    spec: &str,
) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}; run `qusp install <lang> <version>` and create one first", cwd.display()))?;
    let mut m = manifest::load(&root)?;
    let mut lock = lock::Lock::load(&root)?;
    let (name, version) = match spec.rsplit_once('@') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (spec.to_string(), "latest".to_string()),
    };
    let client = http_client()?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let pb = spinner(format!("resolving {name}"));
    let (lang, locked) = orch.add_tool(&mut m, &mut lock, &name, &version, &client).await?;
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

fn cmd_run(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    argv: Vec<String>,
) -> Result<ExitCode> {
    if argv.is_empty() { bail!("usage: qusp run <cmd> [args...]"); }
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd);
    let lock = match root.as_deref() {
        Some(r) => lock::Lock::load(r).unwrap_or_else(|_| lock::Lock::empty()),
        None => lock::Lock::empty(),
    };
    let cmd = &argv[0];
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);

    // 1. Project-pinned tool? Prefer that backend's env.
    let (exe, prefer_lang): (std::path::PathBuf, Option<String>) =
        match orch.find_tool(&lock, cmd) {
            Some((lang, _, bin)) if bin.exists() => (bin, Some(lang)),
            _ => {
                // 2. Maybe it's a toolchain binary like `go` or `python` or `ruby`.
                // Iterate backends; whichever has a bin/<cmd> in its toolchain wins.
                let mut found: Option<(std::path::PathBuf, String)> = None;
                for (id, _backend) in r.iter() {
                    let Some(entry) = lock.backends.get(id) else { continue; };
                    if entry.version.is_empty() { continue; }
                    // backend doesn't expose toolchain bin path directly here;
                    // build_run_env's path_prepend[0] is conventionally the bin dir.
                    let env = match _backend.build_run_env(paths, &entry.version, &cwd) {
                        Ok(e) => e, Err(_) => continue,
                    };
                    if let Some(bin_dir) = env.path_prepend.first() {
                        let candidate = bin_dir.join(cmd);
                        if candidate.exists() {
                            found = Some((candidate, id.to_string()));
                            break;
                        }
                    }
                }
                match found {
                    Some((p, id)) => (p, Some(id)),
                    None => (std::path::PathBuf::from(cmd), None),
                }
            }
        };

    let env = orch.build_run_env(&lock, &cwd, prefer_lang.as_deref())?;
    use std::process::Command;
    let mut child = Command::new(&exe);
    child.args(&argv[1..]);
    let mut path_var = std::ffi::OsString::new();
    for (i, p) in env.path_prepend.iter().enumerate() {
        if i > 0 { path_var.push(":"); }
        path_var.push(p);
    }
    if !path_var.is_empty() { path_var.push(":"); }
    path_var.push(std::env::var_os("PATH").unwrap_or_default());
    child.env("PATH", path_var);
    for (k, v) in env.env { child.env(k, v); }
    let status = child.status().map_err(|e| anyhow!("spawn {}: {e}", exe.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

async fn cmd_x(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    argv: Vec<String>,
) -> Result<ExitCode> {
    if argv.is_empty() { bail!("usage: qusp x <tool> [args...]   (or invoke as `quspx`)"); }
    let cmd = &argv[0];
    let rest = &argv[1..];

    // Route the tool name to a backend.
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let (lang, backend) = orch.route_tool(cmd)?;

    // Pick a toolchain version for this lang. Prefer manifest, fall back to
    // backend.detect_version, fall back to latest installed.
    let cwd = std::env::current_dir()?;
    let toolchain_version = {
        let root = manifest::find_root(&cwd);
        let m_version = root
            .as_deref()
            .and_then(|r| manifest::load(r).ok())
            .and_then(|m| m.languages.get(&lang).and_then(|s| s.version.clone()));
        match m_version {
            Some(v) => v,
            None => match backend.detect_version(&cwd).await? {
                Some(d) => d.version,
                None => {
                    let installed = backend.list_installed(paths).unwrap_or_default();
                    installed.into_iter().next().ok_or_else(|| anyhow!(
                        "no {lang} toolchain installed; run `qusp install {lang} <version>` first"
                    ))?
                }
            },
        }
    };

    let client = http_client()?;
    let pb = spinner(format!("resolving {cmd}"));
    let resolved = backend
        .resolve_tool(&client, cmd, &qusp_core::backend::ToolSpec::Short("latest".into()))
        .await?;
    pb.finish_and_clear();
    let pb = spinner(format!("ensuring {}@{} for ephemeral run", resolved.name, resolved.version));
    let locked = backend.install_tool(paths, &toolchain_version, &resolved).await?;
    pb.finish_and_clear();

    let bin = backend.tool_bin_path(paths, &locked);
    let env = backend.build_run_env(paths, &toolchain_version, &cwd)?;
    use std::process::Command;
    let mut child = Command::new(&bin);
    child.args(rest);
    let mut path_var = std::ffi::OsString::new();
    for (i, p) in env.path_prepend.iter().enumerate() {
        if i > 0 { path_var.push(":"); }
        path_var.push(p);
    }
    if !path_var.is_empty() { path_var.push(":"); }
    path_var.push(std::env::var_os("PATH").unwrap_or_default());
    child.env("PATH", path_var);
    for (k, v) in env.env { child.env(k, v); }
    let status = child.status().map_err(|e| anyhow!("spawn {}: {e}", bin.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

async fn cmd_list(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    lang: &str,
    remote: bool,
) -> Result<ExitCode> {
    let backend = r
        .get(lang)
        .ok_or_else(|| anyhow!("unknown language: {lang}"))?;
    if remote {
        let client = http_client()?;
        for v in backend.list_remote(&client).await? {
            println!("{v}");
        }
    } else {
        for v in backend.list_installed(paths)? {
            println!("{v}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_current(r: &BackendRegistry, lang: Option<&str>) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let langs: Vec<&str> = match lang {
        Some(l) => vec![l],
        None => r.ids().collect(),
    };
    for id in langs {
        let backend = match r.get(id) {
            Some(b) => b,
            None => {
                eprintln!("unknown language: {id}");
                continue;
            }
        };
        match backend.detect_version(&cwd).await? {
            Some(d) => println!(
                "{:<10} {} {}",
                color_cyan(id),
                color_bold(&d.version),
                dim(&format!("(from {})", d.source))
            ),
            None => println!("{:<10} {}", color_cyan(id), dim("(none)")),
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_tree(r: &BackendRegistry, paths: &qusp_core::Paths) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let project_root = manifest::find_root(&cwd);
    println!("{}", color_bold("qusp tree"));
    if let Some(root) = project_root.as_deref() {
        let m = manifest::load(root).unwrap_or_default();
        let _ = lock::Lock::load(root); // not yet rendered; reserved for v0.0.2
        if !m.languages.is_empty() {
            println!(
                "├── {} {}",
                color_cyan("manifest"),
                root.join("qusp.toml").display()
            );
            for (lang, sec) in &m.languages {
                let v = sec.version.as_deref().unwrap_or("(no version)");
                println!("│   ├── {} = {}", lang, color_bold(v));
                for (tname, tspec) in &sec.tools {
                    println!("│   │   └── tool {} = \"{}\"", tname, tspec.version());
                }
            }
        } else {
            println!(
                "├── {} {}",
                color_cyan("manifest"),
                dim("(qusp.toml is empty)")
            );
        }
    } else {
        println!(
            "├── {} {}",
            color_cyan("manifest"),
            dim("(no qusp.toml in this tree)")
        );
    }
    println!("└── {} per-language detection", color_cyan("resolution"));
    let last_id = r.ids().last();
    for (id, backend) in r.iter() {
        let prefix = if Some(id) == last_id {
            "    └──"
        } else {
            "    ├──"
        };
        let det = backend.detect_version(&cwd).await?;
        match det {
            Some(d) => println!(
                "{prefix} {} {} {}",
                color_cyan(id),
                color_bold(&d.version),
                dim(&format!("(from {})", d.source))
            ),
            None => println!("{prefix} {} {}", color_cyan(id), dim("(no pin detected)")),
        }
    }
    let _ = paths;
    Ok(ExitCode::SUCCESS)
}

fn cmd_doctor(r: &BackendRegistry, paths: &qusp_core::Paths) -> Result<ExitCode> {
    println!("{}", color_bold("qusp doctor"));
    println!("  data dir   : {}", paths.data.display());
    println!("  config dir : {}", paths.config.display());
    println!("  cache dir  : {}", paths.cache.display());
    println!("  backends   : {}", r.ids().collect::<Vec<_>>().join(", "));
    for (id, backend) in r.iter() {
        let count = backend.list_installed(paths).map(|v| v.len()).unwrap_or(0);
        let mark = if count > 0 {
            color_green(&format!("{count} installed"))
        } else {
            color_yellow("none installed yet").to_string()
        };
        println!("  {:<10} : {mark}", color_cyan(id));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_dir(paths: &qusp_core::Paths, kind: DirKind) -> Result<ExitCode> {
    let p = match kind {
        DirKind::Data => paths.data.clone(),
        DirKind::Cache => paths.cache.clone(),
        DirKind::Config => paths.config.clone(),
    };
    println!("{}", p.display());
    Ok(ExitCode::SUCCESS)
}

fn cmd_completions(shell: clap_complete::Shell) -> Result<ExitCode> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let bin = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(ExitCode::SUCCESS)
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("qusp/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

#[allow(dead_code)]
fn _silence_unused_when_v0_0_1(_: &Path) {}
