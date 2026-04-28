# qusp — Design

> Goals, non-goals, and the architecture that earns the Greg Egan name.

## Mission

`qusp` is the **single CLI that every-day multi-language developers reach
for** when they want toolchain orchestration without the Nix learning
curve. It treats each language with the depth of a single-language tool
(`gv` for Go, `rv` for Ruby, `uv` for Python) while letting one project
manifest declare the whole stack.

## Non-goals

- Replacing Nix. Nix users want bit-perfect reproducibility, including OS
  libraries. `qusp` happily defers to the OS for libc and friends.
- Replacing language-native managers. `cargo`, `pnpm`, `bundler`, `pip`,
  `go install` — these stay canonical for *intra-language* dependency
  graphs. `qusp` only handles toolchain version + global tools.
- Becoming a build system. `cargo run`, `go test`, `bundle exec` aren't
  reinvented; `qusp run` just hands off with the right environment.

## The qusp manifest

```toml
# qusp.toml — at the project root

[go]
version = "1.26.2"

[go.tools]
gopls = "latest"
golangci-lint = "^v1.64"

[ruby]
version = "3.3.5"

[ruby.tools]
rubocop = "latest"

[python]
version = "3.12"
# tools delegated to uv via pyproject.toml — qusp doesn't second-guess uv

[node]
version = "lts"

[node.tools]
eslint = "latest"
prettier = "latest"

[terraform]
version = "1.9.5"

[deno]
version = "2"

[java]
version = "temurin-21"
```

## qusp.lock

```toml
version = 1

[[backend]]
id = "go"
version = "1.26.2"
sha256 = "..."

[[backend.tool]]
name = "gopls"
package = "golang.org/x/tools/gopls"
version = "v0.18.1"
module_hash = "h1:..."

[[backend]]
id = "ruby"
version = "3.3.5"

[[backend.tool]]
name = "rubocop"
gem = "rubocop"
version = "1.65.0"
gem_sha256 = "..."

[[backend]]
id = "python"
version = "3.12.7"
# python tools live in pyproject.toml + uv.lock; qusp just records the
# resolved interpreter version.
```

## Resolution chain

For each language, in priority order:

1. `<APP>_VERSION` env var (e.g. `QUSP_GO_VERSION`)
2. `qusp.toml` `[<lang>] version = "..."`
3. Per-language manifest: `go.mod` toolchain line, `Gemfile`'s `ruby
   "..."` directive, `pyproject.toml` `requires-python`,
   `package.json` `engines.node`, `.terraform-version`, etc.
4. Per-language version file: `.go-version`, `.ruby-version`,
   `.python-version`, `.nvmrc`
5. `~/.config/qusp/global.<lang>`
6. Latest installed for that language
7. (with Nix L2) `flake.nix` declared version

`qusp current` shows which tier won and why, like `gv current` /
`rv current` already do.

## Backend trait

Each language backend implements:

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    /// Stable id ("go", "ruby", "python", "node", "terraform", "deno", "java").
    fn id(&self) -> &'static str;

    /// Files this backend reads when walking up from cwd, in priority order.
    fn manifest_files(&self) -> &[&'static str];

    /// Detect the version pinned by manifests. Returns None if no source pins one.
    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>>;

    /// Install a toolchain version into the qusp store.
    async fn install(&self, paths: &Paths, version: &str) -> Result<InstallReport>;

    /// Drop a toolchain.
    fn uninstall(&self, paths: &Paths, version: &str) -> Result<()>;

    /// List installed versions for this backend.
    fn list_installed(&self, paths: &Paths) -> Result<Vec<String>>;

    /// List remote (installable) versions.
    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>>;

    // ---- tools ----

    /// Resolve a tool spec (name + version range) against the backend's
    /// registry / package server.
    async fn resolve_tool(&self, client: &reqwest::Client, spec: &ToolSpec) -> Result<ResolvedTool>;

    /// Install a tool against a specific toolchain version.
    async fn install_tool(&self, paths: &Paths, version: &str, tool: &ResolvedTool) -> Result<LockedTool>;

    /// Locate a tool's executable path. Used by `qusp run`.
    fn tool_bin_path(&self, paths: &Paths, locked: &LockedTool) -> PathBuf;

    // ---- run env ----

    /// Build the env (PATH, GOROOT, GEM_PATH, etc.) for `qusp run`.
    fn build_run_env(&self, paths: &Paths, version: &str, cwd: &Path) -> Result<RunEnv>;
}
```

Backends today: **`qusp-backend-go`** (wraps `gv-core`),
**`qusp-backend-python`** (subprocess wrapper around `uv`).

Backends planned: ruby (wraps `rv-core`), terraform, deno, node, java.

## Python backend = `uv` delegate

`qusp` does **not** reimplement what `uv` already does well. The Python
backend is a thin wrapper:

| qusp call | becomes |
|---|---|
| `qusp install python 3.12` | `uv python install 3.12` |
| `qusp run python script.py` | `uv run python script.py` |
| `quspx ruff check` (when tool is python) | `uvx ruff check` |
| `qusp tree` (python section) | reads `pyproject.toml` + `uv.lock` |

This makes `qusp` an **integrator, not a competitor** of uv. Python users
keep all of uv's polish; multi-language users get a single CLI without
learning two tools.

## Nix interop roadmap

Four levels of Nix friendliness:

- **L0 — Nix-absent**: works fine without any Nix install. Default.
- **L1 — coexist**: when `/nix/store` exists, prefer Nix substitutes for
  matching toolchain versions. Saves bandwidth and disk for Nix users
  who happen to use qusp at the casual layer.
- **L2 — read flake.nix**: include the project's `flake.nix` (or
  `shell.nix`) in the resolution chain. If the flake declares
  `pkgs.go_1_26`, qusp picks it up without a duplicate `qusp.toml`
  entry.
- **L3 — export to flake.nix**: `qusp export nix` writes a `flake.nix`
  from the current `qusp.toml` + `qusp.lock`. The graduation path: a
  team starts with qusp, scales into Nix.

L2 and L3 differentiate `qusp` from every other casual manager. They're
optional — a user who never touches Nix is unaffected.

## CLI surface

```
qusp init                       create qusp.toml
qusp add <lang> <version>       pin a toolchain
qusp add tool <name>            pin a tool (lang inferred from registry)
qusp install [<lang>] [<ver>]   install everything pinned, or one
qusp uninstall <lang> <ver>
qusp list [--remote] [<lang>]
qusp current [<lang>]           which version resolves + why
qusp which <name>
qusp use-global <lang> <ver>
qusp run <cmd> [args...]        run with full multi-lang env
qusp x / quspx <tool> [args...] ephemeral
qusp sync [--frozen]            install per qusp.lock
qusp lock                       refresh qusp.lock without installing
qusp tree                       multi-lang resolved env
qusp outdated                   drift report
qusp upgrade [<name>] [--all]
qusp tool list / registry / remove
qusp cache info / prune
qusp dir <kind>
qusp env [--shell]              shell-evaluable exports
qusp self-update [--check]
qusp completions <shell>
qusp doctor
qusp export nix                 [v0.5.0] write flake.nix
```

`anyv-core::argv0::rewrite_for_x_dispatch("qusp")` handles `quspx → qusp x`.

## Why this is not asdf+1

asdf has hundreds of plugins and zero depth. Each plugin is a bash script
that downloads + extracts. There's no:

- `go.mod` toolchain directive auto-resolution
- `sum.golang.org` h1: hash verification
- `Gemfile`'s `ruby "..."` directive auto-resolution
- `proxy.golang.org` walk-up for tool packages
- per-toolchain isolated GEM_HOME for tools
- shim with sub-millisecond exec
- delegation to a peer-grade tool (`uv`) when one exists

`qusp` brings each backend's depth from gv/rv/uv-equivalent quality up
to the multi-language layer. The trait surface forces every backend to
implement *good* per-language semantics, not "download a tarball".
