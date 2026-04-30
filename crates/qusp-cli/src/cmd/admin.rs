use anyhow::{bail, Result};
use anyv_core::presentation::{
    bold as color_bold, cyan as color_cyan, green as color_green, spinner, success_mark,
    yellow as color_yellow,
};
use anyv_core::say;
use qusp_core::registry::BackendRegistry;
use std::path::Path;
use std::process::ExitCode;

use crate::output::{self, OutputFormat};

const DEFAULT_VERSIONS: &[(&str, &str)] = &[
    ("go", "1.26.2"),
    ("ruby", "3.4.7"),
    ("python", "3.14.4"),
    ("node", "22.9.0"),
    ("deno", "2.7.14"),
    ("bun", "1.3.13"),
    ("java", "21"),
    ("kotlin", "2.3.21"),
    ("rust", "1.95.0"),
    ("zig", "0.16.0"),
    ("julia", "1.12.6"),
    ("crystal", "1.20.0"),
    ("groovy", "4.0.22"),
    ("dart", "3.5.4"),
    ("scala", "3.8.3"),
    ("clojure", "1.12.4.1618"),
    ("lua", "5.4.7"),
    ("haskell", "9.10.1"),
];

fn default_example_version(id: &str) -> &'static str {
    DEFAULT_VERSIONS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, v)| *v)
        .unwrap_or("<version>")
}

pub fn cmd_init(r: &BackendRegistry, langs: Option<Vec<String>>, force: bool) -> Result<ExitCode> {
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
            out.push_str(&format!("# [{id}]\n"));
            out.push_str(&format!("# version = \"{}\"\n", default_example_version(id)));
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
            out.push_str(&format!("[{id}]\n"));
            out.push_str(&format!("version = \"{}\"\n", default_example_version(id)));
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

pub async fn cmd_self_update(check: bool) -> Result<ExitCode> {
    let updater = anyv_core::selfupdate::SelfUpdate {
        repo: "O6lvl4/qusp",
        bin_name: "qusp",
        current_version: env!("CARGO_PKG_VERSION"),
    };
    let live = super::http()?;
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

pub fn cmd_setup() -> Result<ExitCode> {
    let paths_d = Path::new("/etc/paths.d/qusp");
    let farm_dir = std::env::var("HOME")
        .map(|h| format!("{h}/.local/bin"))
        .unwrap_or_else(|_| String::from("/usr/local/bin"));

    if paths_d.is_file() {
        let content = std::fs::read_to_string(paths_d).unwrap_or_default();
        if content.trim() == farm_dir {
            say!(
                "{} /etc/paths.d/qusp already configured → {}",
                success_mark(),
                color_green(&farm_dir)
            );
            say!("  All apps (VSCode, Terminal, etc.) will see qusp-managed tools.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    say!("{}", color_bold("qusp setup"));
    say!("");
    say!(
        "  This creates {} containing:",
        color_cyan("/etc/paths.d/qusp")
    );
    say!("  {}", color_green(&farm_dir));
    say!("");
    say!("  This ensures all macOS apps (including VSCode opened from Dock)");
    say!("  can find qusp-managed tools on PATH.");
    say!("");
    say!("  Requires {} (one-time). Run:", color_bold("sudo"));
    say!("");
    say!("    sudo sh -c 'echo {} > /etc/paths.d/qusp'", farm_dir);
    say!("");

    match std::fs::write(paths_d, format!("{farm_dir}\n")) {
        Ok(()) => {
            say!(
                "{} /etc/paths.d/qusp created. Restart apps to pick up the new PATH.",
                success_mark()
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            say!(
                "{}",
                color_yellow("  (permission denied — run the sudo command above)")
            );
            Ok(ExitCode::from(1))
        }
        Err(e) => Err(e.into()),
    }
}

pub fn cmd_doctor(
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

    if fmt != OutputFormat::Text {
        return Ok(ExitCode::SUCCESS);
    }

    say!("");

    let farm_dir = std::env::var("HOME")
        .map(|h| format!("{h}/.local/bin"))
        .unwrap_or_default();
    let on_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|p| p == farm_dir);
    if on_path {
        say!("{} ~/.local/bin on PATH", success_mark());
    } else {
        say!(
            "{} ~/.local/bin is NOT on PATH — add it to your shell rc",
            color_yellow("!")
        );
    }

    #[cfg(target_os = "macos")]
    {
        let paths_d = Path::new("/etc/paths.d/qusp");
        if paths_d.is_file() {
            let content = std::fs::read_to_string(paths_d).unwrap_or_default();
            if content.trim().contains(".local/bin") {
                say!(
                    "{} /etc/paths.d/qusp configured (VSCode/GUI apps can see qusp tools)",
                    success_mark()
                );
            } else {
                say!(
                    "{} /etc/paths.d/qusp exists but doesn't contain ~/.local/bin",
                    color_yellow("!")
                );
            }
        } else {
            say!(
                "{} /etc/paths.d/qusp missing — GUI apps (VSCode from Dock) won't see qusp tools",
                color_yellow("!")
            );
            say!("  → run {} to fix", color_cyan("qusp setup"));
        }
    }

    let pins = qusp_core::effects::GlobalPins::load(&paths.config).unwrap_or_default();
    if pins.pins.is_empty() {
        say!(
            "{} no global pins set (run {} to expose bare commands)",
            color_yellow("!"),
            color_cyan("qusp pin set <lang> <ver>")
        );
    } else {
        say!(
            "{} {} global pin(s) active",
            success_mark(),
            pins.pins.len()
        );
    }

    Ok(ExitCode::SUCCESS)
}

pub fn cmd_dir(
    paths: &qusp_core::Paths,
    kind: crate::DirKind,
    fmt: OutputFormat,
) -> Result<ExitCode> {
    let p = match kind {
        crate::DirKind::Data => paths.data.clone(),
        crate::DirKind::Cache => paths.cache.clone(),
        crate::DirKind::Config => paths.config.clone(),
    };
    let out = output::DirOutput {
        kind: format!("{kind:?}").to_lowercase(),
        path: output::path_to_string(&p),
    };
    fmt.emit(&out);
    Ok(ExitCode::SUCCESS)
}
