use anyhow::{anyhow, bail, Result};
use anyv_core::presentation::spinner;
use qusp_core::registry::BackendRegistry;
use qusp_core::{lock, manifest};
use std::process::ExitCode;

pub fn cmd_run(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    argv: Vec<String>,
) -> Result<ExitCode> {
    if argv.is_empty() {
        bail!("usage: qusp run <cmd> [args...]");
    }
    let cwd = std::env::current_dir()?;
    let root = manifest::find_root(&cwd);
    let mut lock = match root.as_deref() {
        Some(r) => lock::Lock::load(r).unwrap_or_else(|_| lock::Lock::empty()),
        None => lock::Lock::empty(),
    };
    backfill_lock_from_manifest(&mut lock, root.as_deref());
    let cmd = &argv[0];
    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);

    let (exe, prefer_lang) = resolve_run_target(r, &orch, paths, &lock, cmd, &cwd)?;
    let env = orch.build_run_env(&lock, &cwd, prefer_lang.as_deref())?;
    exec_with_env(&exe, &argv[1..], &env)
}

fn backfill_lock_from_manifest(lock: &mut lock::Lock, root: Option<&std::path::Path>) {
    let Some(root) = root else { return };
    let Ok(m) = manifest::load(root) else { return };
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

fn resolve_run_target(
    r: &BackendRegistry,
    orch: &qusp_core::orchestrator::Orchestrator,
    paths: &qusp_core::Paths,
    lock: &lock::Lock,
    cmd: &str,
    cwd: &std::path::Path,
) -> Result<(std::path::PathBuf, Option<String>)> {
    if let Some((lang, _, bin)) = orch.find_tool(lock, cmd) {
        if bin.exists() {
            return Ok((bin, Some(lang)));
        }
    }
    for (id, backend) in r.iter() {
        let Some(entry) = lock.backends.get(id) else {
            continue;
        };
        if entry.version.is_empty() {
            continue;
        }
        let env = match backend.build_run_env(paths, &entry.version, cwd) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if let Some(bin_dir) = env.path_prepend.first() {
            let candidate = bin_dir.join(cmd);
            if candidate.exists() {
                return Ok((candidate, Some(id.to_string())));
            }
        }
    }
    Ok((std::path::PathBuf::from(cmd), None))
}

pub async fn cmd_x(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    argv: Vec<String>,
) -> Result<ExitCode> {
    if argv.is_empty() {
        bail!("usage: qusp x <tool|script> [args...]   (or invoke as `quspx`)");
    }
    let cmd = &argv[0];
    let rest = &argv[1..];

    match crate::script::detect_script_invocation(cmd) {
        crate::script::ScriptInvocation::Routed(script_path, lang) => {
            return crate::script::run_script(r, paths, &script_path, lang, rest).await;
        }
        crate::script::ScriptInvocation::UnsupportedExtension(path) => {
            bail!("{}", crate::script::unsupported_extension_message(&path));
        }
        crate::script::ScriptInvocation::NotAFile => {}
    }

    let orch = qusp_core::orchestrator::Orchestrator::new(r, paths);
    let (lang, backend) = orch.route_tool(cmd)?;

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

    let client = super::http()?;
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
    exec_with_env(&bin, rest, &env)
}

fn exec_with_env(
    exe: &std::path::Path,
    args: &[String],
    env: &qusp_core::RunEnv,
) -> Result<ExitCode> {
    use std::process::Command;
    let mut child = Command::new(exe);
    child.args(args);
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
    for (k, v) in &env.env {
        child.env(k, v);
    }
    let status = child
        .status()
        .map_err(|e| anyhow!("spawn {}: {e}", exe.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}
