//! qusp CLI — v0.30.0.
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

use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Result;
use anyv_core::presentation::set_quiet;
use clap::{Parser, Subcommand};
use qusp_core::backends;
use qusp_core::registry::BackendRegistry;
use qusp_core::paths;

mod cmd;
mod output;
mod script;

use cmd::pin::PinCmd;
use cmd::shell::ShellKind;
use output::OutputFormat;

#[derive(Debug, Parser)]
#[command(
    name = "qusp",
    version,
    about = "Every language toolchain in superposition. `cd` collapses to one.",
    propagate_version = true
)]
pub(crate) struct Cli {
    #[arg(short = 'q', long = "quiet", global = true)]
    quiet: bool,
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
    /// project. Source via `eval "$(qusp shellenv)"`.
    Shellenv {
        #[arg(long, value_enum, default_value_t = ShellKind::Auto)]
        shell: ShellKind,
        #[arg(long)]
        root: Option<std::path::PathBuf>,
    },
    /// Print a shell hook that re-applies `qusp shellenv` on every
    /// directory change.
    Hook {
        #[arg(long, value_enum, default_value_t = ShellKind::Auto)]
        shell: ShellKind,
    },
    /// Scaffold a starter `qusp.toml` in the current directory.
    Init {
        #[arg(long, value_delimiter = ',')]
        langs: Option<Vec<String>>,
        #[arg(long)]
        force: bool,
    },
    /// Report toolchains/tools that have newer versions upstream.
    Outdated,
    /// In-place self-update against the latest GitHub release.
    SelfUpdate {
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
    /// One-time system setup: ensure ~/.local/bin is visible to all apps.
    Setup,
    /// Set, list, or remove the global pin for a language.
    Pin {
        #[command(subcommand)]
        cmd: PinCmd,
    },
}

#[derive(Debug, Subcommand)]
enum AddCmd {
    /// `qusp add tool gopls` — auto-detected as a Go tool.
    Tool { spec: String },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum DirKind {
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
    let fmt = cli.output_format;

    match cli.cmd {
        Cmd::Backends => cmd::query::cmd_backends(&registry, fmt),
        Cmd::Install { lang, version } => cmd::install::cmd_install(&registry, &paths, lang, version).await,
        Cmd::Sync { frozen } => cmd::install::cmd_sync(&registry, &paths, frozen).await,
        Cmd::Add { target } => match target {
            AddCmd::Tool { spec } => cmd::install::cmd_add_tool(&registry, &paths, &spec).await,
        },
        Cmd::Run { argv } => cmd::run::cmd_run(&registry, &paths, argv),
        Cmd::X { argv } => cmd::run::cmd_x(&registry, &paths, argv).await,
        Cmd::Shellenv { shell, root } => cmd::shell::cmd_shellenv(&registry, &paths, shell, root),
        Cmd::Hook { shell } => cmd::shell::cmd_hook(shell),
        Cmd::Init { langs, force } => cmd::admin::cmd_init(&registry, langs, force),
        Cmd::Outdated => cmd::query::cmd_outdated(&registry, fmt).await,
        Cmd::SelfUpdate { check } => cmd::admin::cmd_self_update(check).await,
        Cmd::List { lang, remote } => cmd::query::cmd_list(&registry, &paths, &lang, remote, fmt).await,
        Cmd::Current { lang } => cmd::query::cmd_current(&registry, lang.as_deref(), fmt).await,
        Cmd::Tree => cmd::query::cmd_tree(&registry, &paths).await,
        Cmd::Doctor => cmd::admin::cmd_doctor(&registry, &paths, fmt),
        Cmd::Dir { kind } => cmd::admin::cmd_dir(&paths, kind, fmt),
        Cmd::Completions { shell } => cmd_completions(shell),
        Cmd::Setup => cmd::admin::cmd_setup(),
        Cmd::Pin { cmd } => cmd::pin::cmd_pin(&registry, &paths, cmd, fmt).await,
    }
}

fn build_registry() -> BackendRegistry {
    let mut r = BackendRegistry::new();
    r.register(Arc::new(backends::go::GoBackend));
    r.register(Arc::new(backends::ruby::RubyBackend));
    r.register(Arc::new(backends::python::PythonBackend));
    r.register(Arc::new(backends::node::NodeBackend));
    r.register(Arc::new(backends::php::PhpBackend));
    r.register(Arc::new(backends::deno::DenoBackend));
    r.register(Arc::new(backends::elm::ElmBackend));
    r.register(Arc::new(backends::gleam::GleamBackend));
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

fn cmd_completions(shell: clap_complete::Shell) -> Result<ExitCode> {
    let mut cmd = <Cli as clap::CommandFactory>::command();
    let bin = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(ExitCode::SUCCESS)
}
