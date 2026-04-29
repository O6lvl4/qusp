# qusp

> Every language toolchain in superposition. `cd` collapses to one.

**`qusp` is a multi-language toolchain manager.** One `qusp.toml` describes the
toolchains a project needs; qusp resolves and installs them all in parallel,
with reproducibility locked in `qusp.lock`. Every backend is **native Rust** —
no plugin bash, no subprocess freeloading on `rustup` / `nvm` / `pyenv`.

The pitch: **uv-grade quality, every language**, with three usage styles —
`qusp run`, `mise activate`-style hooks, or a **symlink farm** that makes
`python`, `cargo`, `node` "just work" system-wide without any shell hook.

The name comes from Greg Egan's *Schild's Ladder*: a **qusp** is a
quantum-superposition processor. That's what a project's toolchain feels like
— many candidate versions in superposition, until you `cd` and the manifest
collapses the wavefunction.

## Install

### Homebrew (macOS / Linux)

```bash
brew install O6lvl4/tap/qusp
```

### Cargo

```bash
cargo install --git https://github.com/O6lvl4/qusp --tag v0.29.1 --bin qusp
```

### Pre-built binaries

Each release ships sha256-verified tarballs for `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`,
and `x86_64-pc-windows-msvc`. See [Releases].

[Releases]: https://github.com/O6lvl4/qusp/releases

## 30-second quickstart

```bash
# --- per-project workflow ---
qusp init                            # writes a starter qusp.toml
qusp install go 1.26.2               # pin one language
qusp install rust stable             # channel resolution (rustup-compatible)
qusp install                         # install everything in qusp.toml, in parallel
qusp run go test ./...               # run with the project-pinned toolchain

# --- system-wide (no shell hook needed) ---
qusp install python 3.11.13          # install to qusp store
qusp pin set python 3.11.13          # expose python, pip in ~/.local/bin/
python --version                     # → Python 3.11.15 (bare command, no activation)

qusp pin list                        # show all global pins
qusp doctor                          # health check (PATH, VSCode integration, …)
qusp setup                           # one-time: make GUI apps see qusp tools
```

## Three modes, one tool

### 1. Symlink farm (recommended for daily use)

`qusp install` + `qusp pin set` places symlinks in `~/.local/bin/` — the
same model uv uses for `python3.13`. No shim, no shell hook, no overhead.
Bare commands work everywhere: terminal, scripts, cron, VSCode.

```bash
$ qusp install node 22.9.0 && qusp pin set node 22.9.0
✓ node 22.9.0 installed
✓ pinned node 22.9.0 globally
  + farm: node, npm, npx, corepack

$ which node
~/.local/bin/node

$ node --version
v22.9.0
```

Run `qusp setup` once to create `/etc/paths.d/qusp` so that GUI apps
(VSCode launched from Dock, etc.) also see qusp tools on PATH.

### 2. uv-style (explicit `qusp run`)

Nothing about your shell changes. Everything goes through `qusp run` / `quspx`.

```bash
$ qusp run python --version
Python 3.13.0

$ quspx pnpm install                 # ephemeral run via argv[0] dispatch
```

### 3. mise-style (opt-in shell hook)

A `chpwd` hook injects PATH + GOROOT + JAVA_HOME + … so toolchains
resolve per-directory, restoring the baseline on `cd` out.

```bash
$ eval "$(qusp hook --shell zsh)"    # add to ~/.zshrc
$ cd ~/projects/myapp
$ which python
~/Library/Application Support/dev.O6lvl4.qusp/python/3.13.0/bin/python
$ cd /tmp
$ which python
/usr/bin/python                       # auto-restored
```

## Languages

19 backends, all native Rust:

| Backend | Source | Verification | Tools |
|---|---|---|---|
| **go** | go.dev official tarballs | sha256 | full `gv` registry (gopls, golangci-lint, …) |
| **ruby** | ruby-lang.org via `ruby-build` | sha256 | bundler, rake (via `rv`) |
| **python** | python-build-standalone | sha256 | — |
| **node** | nodejs.org official | sha256 | pnpm, yarn, tsc, prettier |
| **deno** | denoland/deno releases | sha256 | — |
| **bun** | oven-sh/bun releases | sha256 | — |
| **java** | Foojay disco API (Temurin/Corretto/Zulu/GraalVM CE) | sha256 | mvn (sha512), gradle (sha256) |
| **rust** | static.rust-lang.org | sha256 | — |
| **kotlin** | JetBrains/kotlin releases | sha256 | — (requires `[java]`) |
| **scala** | Coursier | sha256 | — |
| **groovy** | Apache Groovy releases | sha256 | — |
| **clojure** | Clojure releases | sha256 | — |
| **zig** | ziglang.org releases | sha256 | — |
| **julia** | julialang.org releases | sha256 | — |
| **crystal** | crystal-lang.org releases | sha256 | — |
| **dart** | dart.dev releases | sha256 | — |
| **elm** | elm/compiler GitHub releases | content-addressed | — |
| **lua** | lua.org source (compiled locally) | sha256 | — |
| **haskell** | GHCup releases | sha256 | — |

Every install **verifies a publisher-published hash** before extracting.

### Multi-vendor (Java)

```toml
[java]
version = "21"
distribution = "temurin"           # or "corretto" | "zulu" | "graalvm_community"
```

Resolution goes through Foojay disco, the same registry SDKMAN uses.
`qusp pin set java 21` auto-detects the installed distribution.

## How it differs

| | mise / asdf | proto | uv (Python) | sdkman | devbox / Nix | **qusp** |
|---|---|---|---|---|---|---|
| Languages | 100+ via plugins | ~15 | 1 | JVM only | unlimited via Nix | 19 native |
| Plugin model | bash plugins | Rust | n/a | bash | derivations | none — native Rust |
| Hash verification | varies | varies | strict | sha256 | derivation | **strict, every install** |
| Subprocess freeloading | yes | partial | none | yes | none | **none** |
| Per-vendor (Java) | plugin per vendor | n/a | n/a | curated | per-derivation | **first-class via Foojay** |
| Bare commands | shim or shellenv | shim | symlink farm | shellenv | shell-direct | **symlink farm** |
| Lockfile | partial | partial | yes | no | flake.lock | yes (`qusp.lock`) |
| Reproducibility | partial | partial | uv.lock | low | high | **lockfile + content-addressed store** |

**qusp's lane**: deeper than mise/asdf (no plugins, native everywhere,
strict hash verification), broader than uv (every language, not just
Python), and friendlier than Nix (no derivation language). It is **not**
trying to replace Nix for OS-library reproducibility.

### Latency

`scripts/bench.sh` measures invocation cost via [hyperfine] on
macOS-13 x86_64.

[hyperfine]: https://github.com/sharkdp/hyperfine

| Mode | Mean | User+Sys CPU |
|---|---|---|
| `qusp run go version` | **12.0 ms** | 9 ms |
| `mise exec go version` | 12.1 ms | 9 ms |
| `mise shim go version` (default) | 49.4 ms | 39 ms |
| `~/.local/bin/go version` (qusp farm) | **~1 ms** | <1 ms |

The farm approach is the fastest — it's a direct symlink, no resolution
step at all.

## Architecture

- `qusp-core` — `Backend` trait, manifest, lock, orchestrator, symlink farm
- `qusp-cli` — argv[0] dispatch (`qusp` vs `quspx`), command surface
- Substrate: [`anyv-core`](https://github.com/O6lvl4/anyv-core) (paths,
  extract, sha verification, presentation, self-update)
- Direct deps: [`gv-core`](https://github.com/O6lvl4/gv) for Go,
  [`rv-core`](https://github.com/O6lvl4/rv) for Ruby — Cargo libraries,
  not subprocesses

The orchestrator is the only place that fans out across backends. CLI
handlers reduce to `Orchestrator::{install_toolchains, sync, add_tool,
find_tool, build_run_env, route_tool}`. Adding a new language is one
new file under `crates/qusp-core/src/backends/` plus one
`r.register(...)` line.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the trait surface
and design decisions.

## What qusp is **not**

- A package manager. It manages toolchains, not Maven/npm/PyPI artifacts.
- A reproducible-OS environment manager. Use Nix or devbox for that.
- A plugin platform. The strength is curated quality across 19 languages.
- A drop-in replacement for `cargo install` / `npm install -g` / `gem install`
  / `pip install`. Tools that have peer-dep complexity are intentionally
  not in qusp's curated registries.

## Status

**v0.29.1** — 19 languages, symlink farm, global pins, VSCode/GUI integration.

- Symlink farm: `qusp install` + `qusp pin set` exposes bare commands in `~/.local/bin/`
- Global pins: per-language version control for unversioned bare commands
- `qusp setup`: one-time `/etc/paths.d/qusp` for GUI app visibility
- `qusp doctor`: health check with PATH, pins, and integration diagnostics
- Content-addressed store with strict hash verification on every install
- Tested on macOS x86_64 (daily dogfood), CI on macOS arm64 + Linux + Windows

## Roadmap

- **v0.30** — `qusp uninstall` auto-cleans farm links, `qusp doctor` shows
  farm status (linked / foreign / orphan)
- **v1.0** — API freeze, sigstore signature verification, sbom export
- **Later** — Nix L1/L2/L3 interop

## Contributing

This is a single-author project right now. Issues + PRs welcome on
GitHub. Architecture deviations should come with a `docs/RFC-*.md`
proposal that matches the rest of the design philosophy: native-Rust,
strict-verification, no plugin layer.

## License

MIT
