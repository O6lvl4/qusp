use anyhow::Result;
use qusp_core::registry::BackendRegistry;
use qusp_core::{lock, manifest};
use std::collections::BTreeMap;
use std::process::ExitCode;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ShellKind {
    Auto,
    Zsh,
    Bash,
    Fish,
    Pwsh,
}

impl ShellKind {
    pub fn resolve(self) -> Self {
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

pub fn cmd_shellenv(
    r: &BackendRegistry,
    paths: &qusp_core::Paths,
    shell: ShellKind,
    root: Option<std::path::PathBuf>,
) -> Result<ExitCode> {
    let shell = shell.resolve();
    let cwd = std::env::current_dir()?;
    let project_root = root.or_else(|| manifest::find_root(&cwd));
    let Some(project_root) = project_root else {
        return Ok(ExitCode::SUCCESS);
    };
    let manifest = manifest::load(&project_root).unwrap_or_default();
    let mut lock = lock::Lock::load(&project_root).unwrap_or_else(|_| lock::Lock::empty());
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
    let env_keys: Vec<String> = env.env.keys().cloned().collect();
    let root_str = project_root.display().to_string();

    let out = match shell {
        ShellKind::Fish => emit_fish(&path_prepend, &env.env, &env_keys, &root_str),
        ShellKind::Pwsh => emit_pwsh(&path_prepend.join(":"), &env.env, &env_keys, &root_str),
        _ => emit_posix(&path_prepend.join(":"), &env.env, &env_keys, &root_str),
    };
    print!("{out}");
    Ok(ExitCode::SUCCESS)
}

fn emit_fish(
    path_prepend: &[String],
    env: &BTreeMap<String, String>,
    env_keys: &[String],
    root: &str,
) -> String {
    let mut out = String::new();
    for p in path_prepend {
        out.push_str(&format!("set -gx PATH {} $PATH\n", sh_quote_fish(p)));
    }
    for (k, v) in env {
        out.push_str(&format!("set -gx {k} {}\n", sh_quote_fish(v)));
    }
    out.push_str(&format!(
        "set -gx _QUSP_LAST_KEYS {}\n",
        sh_quote_fish(&env_keys.join(":"))
    ));
    out.push_str(&format!(
        "set -gx _QUSP_ACTIVE_ROOT {}\n",
        sh_quote_fish(root)
    ));
    out
}

fn emit_pwsh(
    path_str: &str,
    env: &BTreeMap<String, String>,
    env_keys: &[String],
    root: &str,
) -> String {
    let mut out = String::new();
    if !path_str.is_empty() {
        out.push_str(&format!(
            "$env:PATH = \"{};\" + $env:PATH\n",
            pwsh_escape(&path_str.replace(':', ";"))
        ));
    }
    for (k, v) in env {
        out.push_str(&format!("$env:{k} = \"{}\"\n", pwsh_escape(v)));
    }
    out.push_str(&format!(
        "$env:_QUSP_LAST_KEYS = \"{}\"\n",
        pwsh_escape(&env_keys.join(":"))
    ));
    out.push_str(&format!(
        "$env:_QUSP_ACTIVE_ROOT = \"{}\"\n",
        pwsh_escape(root)
    ));
    out
}

fn emit_posix(
    path_str: &str,
    env: &BTreeMap<String, String>,
    env_keys: &[String],
    root: &str,
) -> String {
    let mut out = String::new();
    if !path_str.is_empty() {
        out.push_str(&format!("export PATH={}:$PATH\n", sh_quote_posix(path_str)));
    }
    for (k, v) in env {
        out.push_str(&format!("export {k}={}\n", sh_quote_posix(v)));
    }
    out.push_str(&format!(
        "export _QUSP_LAST_KEYS={}\n",
        sh_quote_posix(&env_keys.join(":"))
    ));
    out.push_str(&format!(
        "export _QUSP_ACTIVE_ROOT={}\n",
        sh_quote_posix(root)
    ));
    out
}

pub fn cmd_hook(shell: ShellKind) -> Result<ExitCode> {
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
