//! qusp CLI — v0.0.1.
//!
//! Ships Go (via `gv`) and Python (via `uv`) backends as subprocess
//! wrappers. Proves the multi-language manifest end-to-end while we
//! incubate native ruby/terraform/deno/node/java backends.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use anyv_core::presentation::{
    bold as color_bold, cyan as color_cyan, dim, green as color_green, set_quiet, spinner,
    success_mark, yellow as color_yellow,
};
use anyv_core::say;
use clap::{Parser, Subcommand};
use qusp_core::backend::Backend;
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
    /// Install a toolchain version (e.g. `qusp install go 1.26.2`).
    Install { lang: String, version: String },
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
        Cmd::Install { lang, version } => cmd_install(&registry, &paths, &lang, &version).await,
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
    lang: &str,
    version: &str,
) -> Result<ExitCode> {
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
    Ok(ExitCode::SUCCESS)
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
    let gv = std::process::Command::new("gv")
        .arg("--version")
        .output()
        .ok();
    match gv {
        Some(o) if o.status.success() => println!(
            "  gv         : {}",
            String::from_utf8_lossy(&o.stdout).trim()
        ),
        _ => println!(
            "  gv         : {}",
            color_yellow("MISSING — go backend will fail")
        ),
    }
    let uv = std::process::Command::new("uv")
        .arg("--version")
        .output()
        .ok();
    match uv {
        Some(o) if o.status.success() => println!(
            "  uv         : {}",
            String::from_utf8_lossy(&o.stdout).trim()
        ),
        _ => println!(
            "  uv         : {}",
            color_yellow("MISSING — python backend will fail")
        ),
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
