//! qusp CLI — v0.27.0.
//!
//! Native Go/Ruby/Python backends + orchestrator. Two entry-point
//! styles, by design:
//!
//! 1. **uv-style (default).** `qusp run`, `qusp x` / `quspx`. The
//!    global shell is never modified; everything goes through qusp.
//! 2. **mise-style (opt-in).** `eval "$(qusp hook --shell zsh)"`
//!    installs a `chpwd` handler that re-runs `qusp shellenv` to
//!    inject PATH + GOROOT + GEM_HOME + … on entry and restore the
//!    pre-qusp baseline on exit. For people who want `python`,
//!    `go`, `ruby` to "just work" at the bare prompt.

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

mod output;
mod script;

use output::OutputFormat;

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
    /// Output format. `text` (default, human-readable, colored) or
    /// `json` (machine-readable, schema in docs/JSON_SCHEMA.md).
    /// Side-effect commands (`install`, `run`, `x`, `sync`, ...) ignore
    /// this flag — only introspection commands (`backends`, `list`,
    /// `current`, `doctor`, `dir`, `outdated`) honor it.
    #[arg(long = "output-format", value_enum, default_value_t = OutputFormat::Text, global = true)]
    output_format: OutputFormat,
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
    /// Print shell exports (PATH/GOROOT/GEM_HOME/…) for the current
    /// project. Source via `eval "$(qusp shellenv)"`. Designed to be
    /// run from a `cd` hook (see `qusp hook`).
    Shellenv {
        #[arg(long, value_enum, default_value_t = ShellKind::Auto)]
        shell: ShellKind,
        #[arg(long)]
        root: Option<std::path::PathBuf>,
    },
    /// Print a shell hook that re-applies `qusp shellenv` on every
    /// directory change. Designed to be eval'd in your rcfile.
    Hook {
        #[arg(long, value_enum, default_value_t = ShellKind::Auto)]
        shell: ShellKind,
    },
    /// Scaffold a starter `qusp.toml` in the current directory.
    Init {
        /// Languages to include up front. Defaults to no languages
        /// pinned (you `qusp install <lang> <version>` to add them).
        #[arg(long, value_delimiter = ',')]
        langs: Option<Vec<String>>,
        /// Overwrite an existing qusp.toml without asking.
        #[arg(long)]
        force: bool,
    },
    /// Report toolchains/tools that have newer versions upstream than
    /// what's recorded in qusp.lock.
    Outdated,
    /// In-place self-update against the latest GitHub release. Verifies
    /// sha256 before atomic-replacing the running binary.
    SelfUpdate {
        /// Don't write anything — just report whether an update exists.
        #[arg(long)]
        check: bool,
    },
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ShellKind {
    Auto,
    Zsh,
    Bash,
    Fish,
    Pwsh,
}

impl ShellKind {
    fn resolve(self) -> Self {
        if !matches!(self, ShellKind::Auto) {
            return self;
        }
        let shell = std::env::var("SHELL").unwrap_or_default();
        if shell.ends_with("/fish") {
            ShellKind::Fish
        } else if shell.ends_with("/bash") {
            ShellKind::Bash
        } else if shell.ends_with("/pwsh") || shell.ends_with("\\pwsh.exe") {
            ShellKind::Pwsh
        } else {
            ShellKind::Zsh
        }
    }
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
    let fmt = cli.output_format;

    match cli.cmd {
        Cmd::Backends => cmd_backends(&registry, fmt),
        Cmd::Install { lang, version } => cmd_install(&registry, &paths, lang, version).await,
        Cmd::Sync { frozen } => cmd_sync(&registry, &paths, frozen).await,
        Cmd::Add { target } => match target {
            AddCmd::Tool { spec } => cmd_add_tool(&registry, &paths, &spec).await,
        },
        Cmd::Run { argv } => cmd_run(&registry, &paths, argv),
        Cmd::X { argv } => cmd_x(&registry, &paths, argv).await,
        Cmd::Shellenv { shell, root } => cmd_shellenv(&registry, &paths, shell, root),
        Cmd::Hook { shell } => cmd_hook(shell),
        Cmd::Init { langs, force } => cmd_init(&registry, langs, force),
        Cmd::Outdated => cmd_outdated(&registry, fmt).await,
        Cmd::SelfUpdate { check } => cmd_self_update(check).await,
        Cmd::List { lang, remote } => cmd_list(&registry, &paths, &lang, remote, fmt).await,
        Cmd::Current { lang } => cmd_current(&registry, lang.as_deref(), fmt).await,
        Cmd::Tree => cmd_tree(&registry, &paths).await,
        Cmd::Doctor => cmd_doctor(&registry, &paths, fmt),
        Cmd::Dir { kind } => cmd_dir(&paths, kind, fmt),
        Cmd::Completions { shell } => cmd_completions(shell),
    }
}

fn build_registry() -> BackendRegistry {
    let mut r = BackendRegistry::new();
    r.register(Arc::new(backends::go::GoBackend));
    r.register(Arc::new(backends::ruby::RubyBackend));
    r.register(Arc::new(backends::python::PythonBackend));
    r.register(Arc::new(backends::node::NodeBackend));
    r.register(Arc::new(backends::deno::DenoBackend));
    r.register(Arc::new(backends::java::JavaBackend));
    r.register(Arc::new(backends::rust::RustBackend));
    r.register(Arc::new(backends::bun::BunBackend));
    r.register(Arc::new(backends::kotlin::KotlinBackend));
    r.register(Arc::new(backends::zig::ZigBackend));
    r.register(Arc::new(backends::julia::JuliaBackend));
    r.register(Arc::new(backends::crystal::CrystalBackend));
    r.register(Arc::new(backends::groovy::GroovyBackend));
    r.register(Arc::new(backends::dart::DartBackend));
    r.register(Arc::new(backends::scala::ScalaBackend));
    r.register(Arc::new(backends::clojure::ClojureBackend));
    r.register(Arc::new(backends::lua::LuaBackend));
    r.register(Arc::new(backends::haskell::HaskellBackend));
    r
}

fn cmd_backends(r: &BackendRegistry, fmt: OutputFormat) -> Result<ExitCode> {
    let out = output::BackendsOutput {
        backends: r
            .ids()
            .map(|id| output::BackendEntry { id: id.to_string() })
            .collect(),
    };
    fmt.emit(&out);
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
        // For manual `qusp install <lang> <version>` we honour a
        // `qusp.toml` distribution pin if there is one, so vendor stays
        // consistent across project commands.
        let distribution = manifest::find_root(&std::env::current_dir()?)
            .and_then(|root| manifest::load(&root).ok())
            .and_then(|m| m.languages.get(lang).cloned())
            .and_then(|s| s.distribution);
        // Backend's install method drives its own download/build
        // progress via LiveProgress; no outer spinner needed (would
        // race with the per-step bars).
        let opts = qusp_core::InstallOpts { distribution };
        let http = qusp_core::effects::LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))?;
        let progress = qusp_core::effects::LiveProgress::new();
        let report = backend
            .install(paths, version, &opts, &http, &progress)
            .await?;
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
    let pinned = qusp_core::domain::validate(&m, r)?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let started = std::time::Instant::now();
    let result = orch.install_toolchains(&pinned).await?;
    let elapsed = started.elapsed().as_millis();
    say!(
        "{} Installed {} toolchain{} in {}",
        success_mark(),
        result.installed.len(),
        if result.installed.len() == 1 { "" } else { "s" },
        format_duration_ms(elapsed)
    );
    for s in &result.installed {
        let mark = if s.already_present {
            dim("=")
        } else {
            color_green("+")
        };
        let note = if s.already_present {
            dim("(already present)")
        } else {
            dim("(installed)")
        };
        println!(
            " {mark} {} {} {note}",
            color_cyan(&s.lang),
            color_bold(&s.version)
        );
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

async fn cmd_sync(r: &BackendRegistry, paths: &qusp_core::Paths, frozen: bool) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let m = manifest::load(&root)?;
    let pinned = qusp_core::domain::validate(&m, r)?;
    let mut lock = lock::Lock::load(&root)?;
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let client = http()?;
    let started = std::time::Instant::now();
    let summary = orch.sync(&pinned, &mut lock, frozen, &client).await?;
    let elapsed = started.elapsed().as_millis();
    say!(
        "{} Synced {} toolchain{} + {} tool{} in {}",
        success_mark(),
        summary.langs_installed.len(),
        if summary.langs_installed.len() == 1 {
            ""
        } else {
            "s"
        },
        summary.tools_installed.len(),
        if summary.tools_installed.len() == 1 {
            ""
        } else {
            "s"
        },
        format_duration_ms(elapsed)
    );
    for s in &summary.langs_installed {
        let mark = if s.already_present {
            dim("=")
        } else {
            color_green("+")
        };
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
            if summary.tools_removed_from_lock == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    if !summary.langs_failed.is_empty() {
        eprintln!();
        eprintln!(
            "{} {} toolchain{} failed (other backends still installed):",
            color_yellow("!"),
            summary.langs_failed.len(),
            if summary.langs_failed.len() == 1 {
                ""
            } else {
                "s"
            }
        );
        for (lang, err) in &summary.langs_failed {
            eprintln!("  {} {}: {}", color_yellow("✗"), color_cyan(lang), err);
        }
    }
    if !frozen {
        lock.save(&root)?;
        say!(
            "{} wrote {}",
            success_mark(),
            root.join("qusp.lock").display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_add_tool(
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
    let client = http()?;
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

fn cmd_run(r: &BackendRegistry, paths: &qusp_core::Paths, argv: Vec<String>) -> Result<ExitCode> {
    if argv.is_empty() {
        bail!("usage: qusp run <cmd> [args...]");
    }
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd);
    let mut lock = match root.as_deref() {
        Some(r) => lock::Lock::load(r).unwrap_or_else(|_| lock::Lock::empty()),
        None => lock::Lock::empty(),
    };
    // Fall back to manifest pins for any backend that the lock doesn't
    // already cover. Lets `qusp run` work after `qusp install` even
    // without an intermediate `qusp sync`.
    if let Some(root) = root.as_deref() {
        if let Ok(m) = manifest::load(root) {
            for (lang, sec) in &m.languages {
                let Some(v) = sec.version.clone() else {
                    continue;
                };
                let entry = lock.backends.entry(lang.clone()).or_default();
                if entry.version.is_empty() {
                    entry.version = v;
                }
            }
        }
    }
    let cmd = &argv[0];
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);

    // 1. Project-pinned tool? Prefer that backend's env.
    let (exe, prefer_lang): (std::path::PathBuf, Option<String>) = match orch.find_tool(&lock, cmd)
    {
        Some((lang, _, bin)) if bin.exists() => (bin, Some(lang)),
        _ => {
            // 2. Maybe it's a toolchain binary like `go` or `python` or `ruby`.
            // Iterate backends; whichever has a bin/<cmd> in its toolchain wins.
            let mut found: Option<(std::path::PathBuf, String)> = None;
            for (id, _backend) in r.iter() {
                let Some(entry) = lock.backends.get(id) else {
                    continue;
                };
                if entry.version.is_empty() {
                    continue;
                }
                // backend doesn't expose toolchain bin path directly here;
                // build_run_env's path_prepend[0] is conventionally the bin dir.
                let env = match _backend.build_run_env(paths, &entry.version, &cwd) {
                    Ok(e) => e,
                    Err(_) => continue,
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
        if i > 0 {
            path_var.push(":");
        }
        path_var.push(p);
    }
    if !path_var.is_empty() {
        path_var.push(":");
    }
    path_var.push(std::env::var_os("PATH").unwrap_or_default());
    child.env("PATH", path_var);
    for (k, v) in env.env {
        child.env(k, v);
    }
    let status = child
        .status()
        .map_err(|e| anyhow!("spawn {}: {e}", exe.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

async fn cmd_x(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    argv: Vec<String>,
) -> Result<ExitCode> {
    if argv.is_empty() {
        bail!("usage: qusp x <tool|script> [args...]   (or invoke as `quspx`)");
    }
    let cmd = &argv[0];
    let rest = &argv[1..];

    // Hospitality path: if argv[0] is an existing file with a known
    // script extension, run it through the language's canonical
    // single-file runner — installing the toolchain on demand. This
    // is qusp's "uv run hello.py" equivalent, generalized across
    // every backend qusp owns.
    if let Some((script_path, lang)) = script::detect_script_invocation(cmd) {
        return script::run_script(r, paths, &script_path, lang, rest).await;
    }

    // Fall through: argv[0] is a tool name. Route to its backend.
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
                    installed.into_iter().next().ok_or_else(|| {
                        anyhow!(
                        "no {lang} toolchain installed; run `qusp install {lang} <version>` first"
                    )
                    })?
                }
            },
        }
    };

    let client = http()?;
    let pb = spinner(format!("resolving {cmd}"));
    let resolved = backend
        .resolve_tool(
            &client,
            cmd,
            &qusp_core::backend::ToolSpec::Short("latest".into()),
        )
        .await?;
    pb.finish_and_clear();
    let pb = spinner(format!(
        "ensuring {}@{} for ephemeral run",
        resolved.name, resolved.version
    ));
    let locked = backend
        .install_tool(paths, &client, &toolchain_version, &resolved)
        .await?;
    pb.finish_and_clear();

    let bin = backend.tool_bin_path(paths, &locked);
    let env = backend.build_run_env(paths, &toolchain_version, &cwd)?;
    use std::process::Command;
    let mut child = Command::new(&bin);
    child.args(rest);
    let mut path_var = std::ffi::OsString::new();
    for (i, p) in env.path_prepend.iter().enumerate() {
        if i > 0 {
            path_var.push(":");
        }
        path_var.push(p);
    }
    if !path_var.is_empty() {
        path_var.push(":");
    }
    path_var.push(std::env::var_os("PATH").unwrap_or_default());
    child.env("PATH", path_var);
    for (k, v) in env.env {
        child.env(k, v);
    }
    let status = child
        .status()
        .map_err(|e| anyhow!("spawn {}: {e}", bin.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn cmd_shellenv(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    shell: ShellKind,
    root: Option<std::path::PathBuf>,
) -> Result<ExitCode> {
    let shell = shell.resolve();
    let cwd = std::env::current_dir()?;
    let project_root = root.or_else(|| manifest::find_root(&cwd));
    let Some(project_root) = project_root else {
        // No project here — emit nothing. The hook will still strip any
        // previously-applied env so the shell returns to baseline.
        return Ok(ExitCode::SUCCESS);
    };
    let manifest = manifest::load(&project_root).unwrap_or_default();
    let mut lock = lock::Lock::load(&project_root).unwrap_or_else(|_| lock::Lock::empty());
    // Fall back to manifest pins when the lock is empty so `qusp shellenv`
    // works even before `qusp install`/`sync` has been run.
    for (lang, sec) in &manifest.languages {
        let Some(v) = sec.version.clone() else {
            continue;
        };
        let entry = lock.backends.entry(lang.clone()).or_default();
        if entry.version.is_empty() {
            entry.version = v;
        }
    }
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let env = orch.build_run_env(&lock, &project_root, None)?;
    let path_prepend: Vec<String> = env
        .path_prepend
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let path_str = path_prepend.join(":");
    let env_keys: Vec<String> = env.env.keys().cloned().collect();

    let mut out = String::new();
    match shell {
        ShellKind::Fish => {
            for p in &path_prepend {
                out.push_str(&format!("set -gx PATH {} $PATH\n", sh_quote_fish(p)));
            }
            for (k, v) in &env.env {
                out.push_str(&format!("set -gx {k} {}\n", sh_quote_fish(v)));
            }
            out.push_str(&format!(
                "set -gx _QUSP_LAST_KEYS {}\n",
                sh_quote_fish(&env_keys.join(":"))
            ));
            out.push_str(&format!(
                "set -gx _QUSP_ACTIVE_ROOT {}\n",
                sh_quote_fish(&project_root.display().to_string())
            ));
        }
        ShellKind::Pwsh => {
            if !path_str.is_empty() {
                out.push_str(&format!(
                    "$env:PATH = \"{};\" + $env:PATH\n",
                    pwsh_escape(&path_str.replace(':', ";"))
                ));
            }
            for (k, v) in &env.env {
                out.push_str(&format!("$env:{k} = \"{}\"\n", pwsh_escape(v)));
            }
            out.push_str(&format!(
                "$env:_QUSP_LAST_KEYS = \"{}\"\n",
                pwsh_escape(&env_keys.join(":"))
            ));
            out.push_str(&format!(
                "$env:_QUSP_ACTIVE_ROOT = \"{}\"\n",
                pwsh_escape(&project_root.display().to_string())
            ));
        }
        _ => {
            // zsh / bash share POSIX export syntax.
            if !path_str.is_empty() {
                out.push_str(&format!(
                    "export PATH={}:$PATH\n",
                    sh_quote_posix(&path_str)
                ));
            }
            for (k, v) in &env.env {
                out.push_str(&format!("export {k}={}\n", sh_quote_posix(v)));
            }
            out.push_str(&format!(
                "export _QUSP_LAST_KEYS={}\n",
                sh_quote_posix(&env_keys.join(":"))
            ));
            out.push_str(&format!(
                "export _QUSP_ACTIVE_ROOT={}\n",
                sh_quote_posix(&project_root.display().to_string())
            ));
        }
    }
    print!("{out}");
    Ok(ExitCode::SUCCESS)
}

fn cmd_hook(shell: ShellKind) -> Result<ExitCode> {
    let shell = shell.resolve();
    let script = match shell {
        ShellKind::Fish => FISH_HOOK,
        ShellKind::Pwsh => PWSH_HOOK,
        ShellKind::Bash => BASH_HOOK,
        _ => ZSH_HOOK,
    };
    print!("{script}");
    Ok(ExitCode::SUCCESS)
}

const ZSH_HOOK: &str = r#"# qusp zsh hook — eval "$(qusp hook --shell zsh)"
_qusp_apply() {
    if [ -z "${_QUSP_PATH_BASELINE+x}" ]; then
        export _QUSP_PATH_BASELINE="$PATH"
    else
        PATH="$_QUSP_PATH_BASELINE"
    fi
    if [ -n "${_QUSP_LAST_KEYS:-}" ]; then
        local _qk
        for _qk in ${(s.:.)_QUSP_LAST_KEYS}; do
            unset "$_qk"
        done
        unset _QUSP_LAST_KEYS _QUSP_ACTIVE_ROOT
    fi
    eval "$(command qusp shellenv --shell zsh 2>/dev/null)"
}
typeset -ag chpwd_functions
if (( ${chpwd_functions[(I)_qusp_apply]} == 0 )); then
    chpwd_functions+=(_qusp_apply)
fi
_qusp_apply
"#;

const BASH_HOOK: &str = r#"# qusp bash hook — eval "$(qusp hook --shell bash)"
_qusp_apply() {
    if [ -z "${_QUSP_PATH_BASELINE+x}" ]; then
        export _QUSP_PATH_BASELINE="$PATH"
    else
        PATH="$_QUSP_PATH_BASELINE"
    fi
    if [ -n "${_QUSP_LAST_KEYS:-}" ]; then
        local _qk
        for _qk in $(echo "$_QUSP_LAST_KEYS" | tr ':' ' '); do
            unset "$_qk"
        done
        unset _QUSP_LAST_KEYS _QUSP_ACTIVE_ROOT
    fi
    eval "$(command qusp shellenv --shell bash 2>/dev/null)"
}
_qusp_chpwd() {
    if [ "$PWD" != "${_QUSP_LAST_PWD:-}" ]; then
        export _QUSP_LAST_PWD="$PWD"
        _qusp_apply
    fi
}
case ":${PROMPT_COMMAND:-}:" in
    *":_qusp_chpwd:"*) ;;
    *) PROMPT_COMMAND="_qusp_chpwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
esac
_qusp_apply
"#;

const FISH_HOOK: &str = r#"# qusp fish hook — qusp hook --shell fish | source
function _qusp_apply
    if not set -q _QUSP_PATH_BASELINE
        set -gx _QUSP_PATH_BASELINE $PATH
    else
        set -gx PATH $_QUSP_PATH_BASELINE
    end
    if set -q _QUSP_LAST_KEYS; and test -n "$_QUSP_LAST_KEYS"
        for _qk in (string split ':' $_QUSP_LAST_KEYS)
            set -e $_qk
        end
        set -e _QUSP_LAST_KEYS
        set -e _QUSP_ACTIVE_ROOT
    end
    command qusp shellenv --shell fish 2>/dev/null | source
end
function _qusp_on_pwd --on-variable PWD
    _qusp_apply
end
_qusp_apply
"#;

const PWSH_HOOK: &str = r#"# qusp pwsh hook — Invoke-Expression (qusp hook --shell pwsh | Out-String)
function global:_qusp_apply {
    if (-not (Test-Path Env:_QUSP_PATH_BASELINE)) {
        $env:_QUSP_PATH_BASELINE = $env:PATH
    } else {
        $env:PATH = $env:_QUSP_PATH_BASELINE
    }
    if ($env:_QUSP_LAST_KEYS) {
        foreach ($k in $env:_QUSP_LAST_KEYS.Split(':')) {
            if ($k) { Remove-Item -Path "Env:$k" -ErrorAction SilentlyContinue }
        }
        Remove-Item Env:_QUSP_LAST_KEYS -ErrorAction SilentlyContinue
        Remove-Item Env:_QUSP_ACTIVE_ROOT -ErrorAction SilentlyContinue
    }
    $script = (qusp shellenv --shell pwsh 2>$null | Out-String)
    if ($script) { Invoke-Expression $script }
}
$global:_qusp_last_pwd = $null
function global:prompt {
    if ((Get-Location).Path -ne $global:_qusp_last_pwd) {
        $global:_qusp_last_pwd = (Get-Location).Path
        _qusp_apply
    }
    "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) "
}
_qusp_apply
"#;

fn sh_quote_posix(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=+,".contains(c))
    {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn sh_quote_fish(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=+,".contains(c))
    {
        return s.to_string();
    }
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn pwsh_escape(s: &str) -> String {
    s.replace('`', "``").replace('"', "`\"").replace('$', "`$")
}

fn cmd_init(r: &BackendRegistry, langs: Option<Vec<String>>, force: bool) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let target = cwd.join("qusp.toml");
    if target.exists() && !force {
        bail!(
            "qusp.toml already exists at {} — pass --force to overwrite",
            target.display()
        );
    }
    let mut out = String::new();
    out.push_str("# qusp.toml — multi-language toolchain manifest.\n");
    out.push_str(
        "# Pin one section per language; `qusp install` (no args) installs them all in parallel.\n",
    );
    out.push_str("# https://github.com/O6lvl4/qusp\n\n");
    let requested: Vec<String> = langs.unwrap_or_default();
    if requested.is_empty() {
        out.push_str("# Examples (uncomment + adjust):\n");
        for id in r.ids() {
            let example = match id {
                "go" => "1.26.2",
                "ruby" => "3.4.7",
                "python" => "3.13.0",
                "node" => "22.9.0",
                "deno" => "2.0.0",
                "bun" => "1.2.0",
                "java" => "21",
                "kotlin" => "2.1.20",
                "rust" => "1.85.0",
                "zig" => "0.16.0",
                "julia" => "1.10.4",
                "crystal" => "1.20.0",
                "groovy" => "4.0.22",
                "dart" => "3.5.4",
                "scala" => "3.8.3",
                "clojure" => "1.12.4.1618",
                "lua" => "5.4.7",
                "haskell" => "9.10.1",
                _ => "<version>",
            };
            out.push_str(&format!("# [{id}]\n"));
            out.push_str(&format!("# version = \"{example}\"\n"));
            if id == "java" {
                out.push_str("# distribution = \"temurin\"\n");
            }
            out.push('\n');
        }
    } else {
        for id in &requested {
            if r.get(id).is_none() {
                bail!(
                    "unknown language '{id}' (known: {})",
                    r.ids().collect::<Vec<_>>().join(", ")
                );
            }
            let example = match id.as_str() {
                "go" => "1.26.2",
                "ruby" => "3.4.7",
                "python" => "3.13.0",
                "node" => "22.9.0",
                "deno" => "2.0.0",
                "bun" => "1.2.0",
                "java" => "21",
                "kotlin" => "2.1.20",
                "rust" => "1.85.0",
                "zig" => "0.16.0",
                "julia" => "1.10.4",
                "crystal" => "1.20.0",
                "groovy" => "4.0.22",
                "dart" => "3.5.4",
                "scala" => "3.8.3",
                "clojure" => "1.12.4.1618",
                "lua" => "5.4.7",
                "haskell" => "9.10.1",
                _ => "<version>",
            };
            out.push_str(&format!("[{id}]\n"));
            out.push_str(&format!("version = \"{example}\"\n"));
            if id == "java" {
                out.push_str("distribution = \"temurin\"\n");
            }
            out.push('\n');
        }
    }
    std::fs::write(&target, out)?;
    say!("{} wrote {}", success_mark(), target.display());
    Ok(ExitCode::SUCCESS)
}

async fn cmd_outdated(r: &BackendRegistry, fmt: OutputFormat) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd)
        .ok_or_else(|| anyhow!("no qusp.toml found above {}", cwd.display()))?;
    let lock = lock::Lock::load(&root).unwrap_or_else(|_| lock::Lock::empty());
    if lock.backends.is_empty() {
        // Empty lock — emit empty entries; text mode preserves the
        // existing "(qusp.lock has no toolchain entries...)" hint.
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
    let client = http()?;
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
        let pinned = entry.version.trim().to_string();
        let Ok(remote) = remote else {
            entries.push(output::OutdatedEntry {
                backend: lang.clone(),
                status: output::OutdatedStatus::Unknown,
                current: pinned,
                latest: None,
            });
            continue;
        };
        // First entry is the newest by convention. Strip annotations
        // like " (LTS)" / " (current stable)".
        let latest_raw = remote.first().cloned().unwrap_or_default();
        let latest = latest_raw
            .split_whitespace()
            .next()
            .unwrap_or(&latest_raw)
            .to_string();
        if latest.is_empty() {
            continue;
        }
        let status = if version_loose_eq(&pinned, &latest) {
            output::OutdatedStatus::UpToDate
        } else {
            output::OutdatedStatus::Outdated
        };
        entries.push(output::OutdatedEntry {
            backend: lang.clone(),
            status,
            current: pinned,
            latest: Some(latest),
        });
    }
    fmt.emit(&output::OutdatedOutput { entries });
    Ok(ExitCode::SUCCESS)
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

async fn cmd_self_update(check: bool) -> Result<ExitCode> {
    let updater = anyv_core::selfupdate::SelfUpdate {
        repo: "O6lvl4/qusp",
        bin_name: "qusp",
        current_version: env!("CARGO_PKG_VERSION"),
    };
    // anyv-core's SelfUpdate predates qusp's HttpFetcher trait and still
    // wants a raw reqwest::Client. Pull it out of LiveHttp.
    let live = http()?;
    let pb = spinner("checking github.com/O6lvl4/qusp/releases/latest");
    let info = updater.run(live.raw(), check).await?;
    pb.finish_and_clear();
    use anyv_core::selfupdate::Outcome;
    match info.outcome {
        Outcome::AlreadyUpToDate => {
            say!(
                "{} qusp v{} is the latest release",
                success_mark(),
                info.current
            );
        }
        Outcome::NewerAvailable => {
            say!(
                "{} qusp v{} is available (you have v{}). Run without `--check` to install.",
                color_yellow("↑"),
                info.latest,
                info.current
            );
        }
        Outcome::Updated => {
            say!(
                "{} qusp updated v{} → v{} at {}",
                success_mark(),
                info.current,
                info.latest,
                info.binary_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_list(
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
        let client = http()?;
        (output::ListScope::Remote, backend.list_remote(&client).await?)
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

async fn cmd_current(
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

fn cmd_doctor(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    fmt: OutputFormat,
) -> Result<ExitCode> {
    let backends: Vec<output::DoctorBackend> = r
        .iter()
        .map(|(id, backend)| output::DoctorBackend {
            id: id.to_string(),
            installed_count: backend.list_installed(paths).map(|v| v.len()).unwrap_or(0),
        })
        .collect();
    let out = output::DoctorOutput {
        qusp_version: env!("CARGO_PKG_VERSION").to_string(),
        paths: output::DoctorPaths {
            data: output::path_to_string(&paths.data),
            config: output::path_to_string(&paths.config),
            cache: output::path_to_string(&paths.cache),
        },
        backends,
    };
    fmt.emit(&out);
    Ok(ExitCode::SUCCESS)
}

fn cmd_dir(paths: &qusp_core::Paths, kind: DirKind, fmt: OutputFormat) -> Result<ExitCode> {
    let p = match kind {
        DirKind::Data => paths.data.clone(),
        DirKind::Cache => paths.cache.clone(),
        DirKind::Config => paths.config.clone(),
    };
    let out = output::DirOutput {
        kind: format!("{kind:?}").to_lowercase(),
        path: output::path_to_string(&p),
    };
    fmt.emit(&out);
    Ok(ExitCode::SUCCESS)
}

fn cmd_completions(shell: clap_complete::Shell) -> Result<ExitCode> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let bin = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(ExitCode::SUCCESS)
}

/// Build the live HTTP effect handle. Used as `&dyn HttpFetcher` for
/// every Backend/Orchestrator call. anyv-core's SelfUpdate still wants
/// a raw `reqwest::Client`; for that, use `http().raw().clone()`.
fn http() -> Result<qusp_core::effects::LiveHttp> {
    qusp_core::effects::LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))
}

#[allow(dead_code)]
fn _silence_unused_when_v0_0_1(_: &Path) {}
