//! `qusp x <script>` extension-routing — uv-class hospitality for
//! every language qusp ships.
//!
//! When `qusp x` (or its `quspx` shortcut) sees a path-like first
//! argument with a known script extension, it skips the normal
//! tool-dispatch path and instead:
//!
//! 1. Maps the extension to a backend (`hello.lua` → lua).
//! 2. Resolves a version using the same precedence as a manifest
//!    pin (`qusp.toml` > `.lang-version` > newest installed >
//!    qusp's curated default for that language).
//! 3. Installs the toolchain if missing — fully sha-verified, same
//!    as `qusp install <lang> <version>`.
//! 4. Builds the language run-env and `exec`s the canonical script
//!    runner for that language with the script as argv[1..].
//!
//! Net effect: `qusp x ./hello.lua` makes Lua "just work" against
//! a previously-untouched machine, the way `uv run hello.py` does
//! for Python — but for every backend qusp owns.
//!
//! Cross-backend deps from `Backend::requires` are NOT pulled in
//! automatically here. If the user invokes `qusp x ./Hello.scala`
//! and Java isn't installed, Scala's runner will fail with a clear
//! "java not found" — that's the price of `x`'s ephemeral nature
//! (no manifest writes, no transitive resolution). The user can
//! `qusp install java 21` separately, or pin both in `qusp.toml`
//! and use `qusp run`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Result};
use qusp_core::backend::Backend;
use qusp_core::registry::BackendRegistry;
use qusp_core::{manifest, Paths};

/// Map a file's extension to a qusp language id. None means "no
/// extension match" — caller falls through to existing tool dispatch.
pub fn extension_to_lang(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        // Single-source-file scripting languages.
        "py" | "pyi" => "python",
        "lua" => "lua",
        "rb" => "ruby",
        "jl" => "julia",
        "groovy" => "groovy",

        // Source files that ship with a runnable single-file launcher.
        "java" => "java",     // `java Hello.java` (JEP 330, JDK 21+)
        "kts" => "kotlin",    // `kotlin -script Hello.kts`
        "scala" | "sc" => "scala",      // `scala Hello.scala` (3.5+ ships scala-cli)
        "clj" | "cljc" => "clojure",    // `clojure Hello.clj`
        "hs" => "haskell",    // `runghc Hello.hs`

        // JS/TS — choose the simplest defaults; .ts → deno (built-in
        // TypeScript), .js → node (broadest ecosystem). Bun overlaps
        // both; users who want bun pin it via shebang or qusp.toml.
        "js" | "mjs" | "cjs" => "node",
        "ts" | "mts" | "cts" => "deno",

        // Compiled-with-`run`-subcommand languages — they all ship a
        // `<lang> run <file>` mode that compiles + executes
        // transparently for one-shot use.
        "go" => "go",
        "zig" => "zig",
        "dart" => "dart",
        "cr" => "crystal",

        _ => None?,
    })
}

/// The argv (program + args) qusp uses to launch a single script
/// against a pinned-and-installed toolchain. Convention is matched to
/// each language's idiomatic single-file run command.
pub fn script_run_argv(lang: &str, script: &Path) -> Result<Vec<String>> {
    let s = script.to_string_lossy().to_string();
    Ok(match lang {
        "python" => vec!["python".into(), s],
        "lua" => vec!["lua".into(), s],
        "ruby" => vec!["ruby".into(), s],
        "node" => vec!["node".into(), s],
        "deno" => vec!["deno".into(), "run".into(), s],
        "go" => vec!["go".into(), "run".into(), s],
        "java" => vec!["java".into(), s],
        "kotlin" => vec!["kotlin".into(), "-script".into(), s],
        "scala" => vec!["scala".into(), s],
        "clojure" => vec!["clojure".into(), s],
        "haskell" => vec!["runghc".into(), s],
        "zig" => vec!["zig".into(), "run".into(), s],
        "dart" => vec!["dart".into(), "run".into(), s],
        "crystal" => vec!["crystal".into(), "run".into(), s],
        "julia" => vec!["julia".into(), s],
        "groovy" => vec!["groovy".into(), s],
        _ => bail!("internal: no script_run_argv mapping for lang={lang}"),
    })
}

/// qusp's curated default version per language for ephemeral runs.
/// Mirrors the version map used by `qusp init` so a fresh `qusp x
/// hello.lua` and a fresh `qusp init --langs=lua` agree on what
/// "latest reasonable" means at this qusp release.
pub fn default_version(lang: &str) -> Option<&'static str> {
    Some(match lang {
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
        _ => return None,
    })
}

/// Heuristic: argv[0] is a path to a script we should route by
/// extension. Conservative: must (1) parse to a path with an
/// extension we recognise *and* (2) actually exist on disk. Anything
/// less and we fall through to existing tool-name routing.
pub fn detect_script_invocation(argv0: &str) -> Option<(PathBuf, &'static str)> {
    let path = PathBuf::from(argv0);
    let lang = extension_to_lang(&path)?;
    if !path.is_file() {
        return None;
    }
    Some((path, lang))
}

/// Resolve the toolchain version to use for a script run.
/// Precedence:
///   1. **inline script metadata** (`# qusp: lang = X`) — added in v0.26.0
///   2. `qusp.toml`
///   3. `.<lang>-version`
///   4. newest installed
///   5. curated `default_version` table
pub async fn resolve_script_version(
    lang: &str,
    backend: &dyn Backend,
    paths: &Paths,
    script: &Path,
) -> Result<String> {
    if let Some(v) = read_inline_metadata(script, lang) {
        return Ok(v);
    }

    let cwd = std::env::current_dir()?;

    if let Some(root) = manifest::find_root(&cwd) {
        if let Ok(m) = manifest::load(&root) {
            if let Some(v) = m.languages.get(lang).and_then(|s| s.version.clone()) {
                return Ok(v);
            }
        }
    }

    if let Some(d) = backend.detect_version(&cwd).await? {
        return Ok(d.version);
    }

    if let Some(latest) = backend
        .list_installed(paths)
        .unwrap_or_default()
        .into_iter()
        .next()
    {
        return Ok(latest);
    }

    default_version(lang)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("no curated default version for {lang}; pin one in qusp.toml"))
}

/// The line-comment prefixes for each language qusp supports
/// extension-routing for. Used by `read_inline_metadata` to scan a
/// script's prologue. Languages that share a comment syntax share an
/// arm; the order within a slice matters for prefix-stripping (longer
/// before shorter to disambiguate `;;` vs `;` in Clojure).
fn comment_prefixes(lang: &str) -> &'static [&'static str] {
    match lang {
        // Hash-style.
        "python" | "ruby" | "julia" => &["#"],
        // Double-dash.
        "lua" | "haskell" => &["--"],
        // C-style.
        "node" | "deno" | "go" | "scala" | "java" | "kotlin" | "zig"
        | "dart" | "crystal" | "groovy" => &["//"],
        // Lisp-style. Try `;;` first (idiomatic) then bare `;` (single-line).
        "clojure" => &[";;", ";"],
        _ => &[],
    }
}

/// Read inline `# qusp: <lang> = <version>` metadata from the first
/// ~30 lines of a script. Supports per-language comment syntax
/// (`#`, `--`, `//`, `;;`). The directive is case-sensitive on
/// `qusp:` and the language id, but tolerant of:
///
/// - whitespace around `=`
/// - quoted values: `# qusp: python = "3.11.13"` and `'3.11.13'` both work
/// - leading whitespace on the comment itself
///
/// Returns `Some(version)` on first match, `None` otherwise. A
/// language id mismatch (`# qusp: python = X` in a `.lua` script) is
/// silently skipped — the caller is asking about `lang=lua` only.
pub fn read_inline_metadata(script: &Path, lang: &str) -> Option<String> {
    let content = std::fs::read_to_string(script).ok()?;
    let prefixes = comment_prefixes(lang);
    if prefixes.is_empty() {
        return None;
    }
    for line in content.lines().take(30) {
        let trimmed = line.trim_start();
        for prefix in prefixes {
            let Some(rest) = trimmed.strip_prefix(*prefix) else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix("qusp:") else {
                continue;
            };
            let rest = rest.trim();
            // Expected form: "<lang> = <version>"
            let Some((key, value)) = rest.split_once('=') else {
                continue;
            };
            if key.trim() != lang {
                continue;
            }
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Run a script under qusp's ephemeral run path. Installs the
/// toolchain if not already present, then `exec`s the canonical
/// script runner.
pub async fn run_script(
    registry: &BackendRegistry,
    paths: &Paths,
    script: &Path,
    lang: &'static str,
    rest: &[String],
) -> Result<ExitCode> {
    let backend = registry
        .get(lang)
        .ok_or_else(|| anyhow!("internal: no backend registered for lang={lang}"))?;

    let version = resolve_script_version(lang, backend.as_ref(), paths, script).await?;

    // Idempotent install — backend.install short-circuits if the
    // version is already laid out under data/<lang>/<v>.
    let http = qusp_core::effects::LiveHttp::new(concat!("qusp/", env!("CARGO_PKG_VERSION")))?;
    let progress = qusp_core::effects::LiveProgress::new();
    let opts = qusp_core::InstallOpts::default();
    let report = backend
        .install(paths, &version, &opts, &http, &progress)
        .await?;
    if !report.already_present {
        anyv_core::say!(
            "{} {lang} {} installed for ephemeral run",
            anyv_core::presentation::success_mark(),
            report.version
        );
    }

    // Build the run env and dispatch. Compose the same way `qusp run`
    // does for a single-language run.
    let cwd = std::env::current_dir()?;
    let env = backend.build_run_env(paths, &version, &cwd)?;
    let argv = script_run_argv(lang, script)?;
    let (program, args) = argv.split_first().expect("script_run_argv non-empty");

    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.args(rest);

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
    cmd.env("PATH", path_var);
    for (k, v) in env.env {
        cmd.env(k, v);
    }

    let status = cmd
        .status()
        .map_err(|e| anyhow!("spawn {}: {e}", program))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_to_lang_covers_shipped_backends() {
        for (file, lang) in [
            ("hello.py", Some("python")),
            ("hello.lua", Some("lua")),
            ("hello.rb", Some("ruby")),
            ("hello.jl", Some("julia")),
            ("hello.groovy", Some("groovy")),
            ("Hello.java", Some("java")),
            ("Hello.kts", Some("kotlin")),
            ("Hello.scala", Some("scala")),
            ("hello.sc", Some("scala")),
            ("hello.clj", Some("clojure")),
            ("hello.hs", Some("haskell")),
            ("hello.js", Some("node")),
            ("hello.mjs", Some("node")),
            ("hello.cjs", Some("node")),
            ("hello.ts", Some("deno")),
            ("hello.go", Some("go")),
            ("hello.zig", Some("zig")),
            ("hello.dart", Some("dart")),
            ("hello.cr", Some("crystal")),
            ("path/to/HELLO.LUA", Some("lua")),     // case-insensitive
            ("noext", None),
            ("hello.unknown", None),
            ("hello.rs", None),                      // rust scripts NYI
            ("hello.kt", None),                      // .kt needs full compile
        ] {
            assert_eq!(
                extension_to_lang(Path::new(file)),
                lang,
                "ext routing for {file}"
            );
        }
    }

    #[test]
    fn script_run_argv_emits_canonical_run_command() {
        // The lang→argv mapping is the user-facing contract — pinning
        // it as a test prevents accidental drift between qusp releases.
        let p = Path::new("hello.lua");
        assert_eq!(
            script_run_argv("lua", p).unwrap(),
            vec!["lua".to_string(), "hello.lua".to_string()]
        );
        assert_eq!(
            script_run_argv("deno", p).unwrap(),
            vec!["deno".to_string(), "run".to_string(), "hello.lua".to_string()]
        );
        assert_eq!(
            script_run_argv("haskell", p).unwrap(),
            vec!["runghc".to_string(), "hello.lua".to_string()]
        );
        assert_eq!(
            script_run_argv("kotlin", p).unwrap(),
            vec!["kotlin".to_string(), "-script".to_string(), "hello.lua".to_string()]
        );
        assert!(script_run_argv("perl", p).is_err());
    }

    #[test]
    fn default_version_covers_all_extension_langs() {
        // Every language extension_to_lang can return must have a
        // default version; otherwise the user gets "no curated
        // default" mid-flight on a fresh machine.
        for lang in [
            "python", "lua", "ruby", "julia", "groovy", "java", "kotlin",
            "scala", "clojure", "haskell", "node", "deno", "go", "zig",
            "dart", "crystal",
        ] {
            assert!(
                default_version(lang).is_some(),
                "missing default_version for {lang}"
            );
        }
    }

    /// Helper for inline-metadata tests: write `body` to a unique
    /// temp file with the given extension and run `read_inline_metadata`.
    /// Uses pid + atomic counter so parallel test threads can't
    /// clash on the same temp path.
    fn read_md(ext: &str, lang: &str, body: &str) -> Option<String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "qusp-md-{}-{}-{}.{}",
            lang,
            std::process::id(),
            n,
            ext,
        ));
        std::fs::write(&tmp, body).unwrap();
        let result = read_inline_metadata(&tmp, lang);
        std::fs::remove_file(&tmp).ok();
        result
    }

    #[test]
    fn inline_metadata_hash_comment_python() {
        assert_eq!(
            read_md("py", "python", "# qusp: python = \"3.11.13\"\nprint('hi')\n"),
            Some("3.11.13".to_string())
        );
    }

    #[test]
    fn inline_metadata_hash_comment_ruby_unquoted() {
        assert_eq!(
            read_md("rb", "ruby", "# qusp: ruby = 3.4.7\nputs 'hi'\n"),
            Some("3.4.7".to_string())
        );
    }

    #[test]
    fn inline_metadata_double_dash_lua() {
        assert_eq!(
            read_md("lua", "lua", "-- qusp: lua = 5.4.5\nprint('hi')\n"),
            Some("5.4.5".to_string())
        );
    }

    #[test]
    fn inline_metadata_double_dash_haskell() {
        assert_eq!(
            read_md(
                "hs",
                "haskell",
                "-- qusp: haskell = 9.10.1\nmain = putStrLn \"hi\"\n"
            ),
            Some("9.10.1".to_string())
        );
    }

    #[test]
    fn inline_metadata_c_style_scala() {
        assert_eq!(
            read_md(
                "scala",
                "scala",
                "// qusp: scala = 3.8.3\n@main def hi = println(\"hi\")\n"
            ),
            Some("3.8.3".to_string())
        );
    }

    #[test]
    fn inline_metadata_c_style_kotlin_kts() {
        assert_eq!(
            read_md("kts", "kotlin", "// qusp: kotlin = 2.1.20\nprintln(\"hi\")\n"),
            Some("2.1.20".to_string())
        );
    }

    #[test]
    fn inline_metadata_lisp_style_clojure_double_semi() {
        assert_eq!(
            read_md(
                "clj",
                "clojure",
                ";; qusp: clojure = 1.12.4.1618\n(println \"hi\")\n"
            ),
            Some("1.12.4.1618".to_string())
        );
    }

    #[test]
    fn inline_metadata_lisp_style_clojure_single_semi() {
        // Less idiomatic but still valid Clojure comment.
        assert_eq!(
            read_md(
                "clj",
                "clojure",
                "; qusp: clojure = 1.11.4.1474\n(println \"hi\")\n"
            ),
            Some("1.11.4.1474".to_string())
        );
    }

    #[test]
    fn inline_metadata_skips_lang_mismatch() {
        // python script declaring lua version (typo / paste error).
        // Caller asked for python, must NOT confuse and pick lua's version.
        assert_eq!(
            read_md("py", "python", "# qusp: lua = 5.4.5\nprint('hi')\n"),
            None
        );
    }

    #[test]
    fn inline_metadata_returns_none_on_no_match() {
        assert_eq!(read_md("py", "python", "print('plain script')\n"), None);
        assert_eq!(read_md("py", "python", ""), None);
    }

    #[test]
    fn inline_metadata_only_scans_prologue() {
        // The directive on line 50 must be ignored — qusp doesn't parse
        // the entire file (avoids false positives from string literals
        // / data files).
        let mut body = String::new();
        for _ in 0..40 {
            body.push_str("print('filler')\n");
        }
        body.push_str("# qusp: python = 3.10.0\n");
        assert_eq!(read_md("py", "python", &body), None);
    }

    #[test]
    fn inline_metadata_tolerates_leading_whitespace() {
        assert_eq!(
            read_md("py", "python", "    # qusp: python = 3.13.0\n"),
            Some("3.13.0".to_string())
        );
    }

    #[test]
    fn inline_metadata_handles_single_quotes() {
        assert_eq!(
            read_md("py", "python", "# qusp: python = '3.13.0'\n"),
            Some("3.13.0".to_string())
        );
    }

    #[test]
    fn inline_metadata_unsupported_lang_returns_none() {
        // No comment prefix table entry → no match path.
        assert_eq!(read_md("rs", "rust", "// qusp: rust = 1.85.0\n"), None);
    }

    #[test]
    fn detect_script_invocation_requires_existing_file_with_known_ext() {
        // Real file path with known ext — match.
        let tmp = std::env::temp_dir().join(format!(
            "qusp-script-detect-{}.lua",
            std::process::id()
        ));
        std::fs::write(&tmp, "print('hi')\n").unwrap();
        let argv0 = tmp.to_string_lossy().to_string();
        let got = detect_script_invocation(&argv0);
        assert!(got.is_some(), "real .lua should match");
        let (_, lang) = got.unwrap();
        assert_eq!(lang, "lua");

        // Known ext, but file doesn't exist — no match (caller falls
        // through to tool dispatch, gets a "no such tool" error).
        assert!(detect_script_invocation("/tmp/nope-does-not-exist.lua").is_none());

        // Bare command name (looks like a tool) — no match.
        assert!(detect_script_invocation("gopls").is_none());

        std::fs::remove_file(&tmp).ok();
    }
}
