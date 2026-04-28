# qusp

> Every language toolchain in superposition. `cd` collapses to one.

**`qusp` is a multi-language toolchain manager.** One `qusp.toml` describes the
toolchains a project needs (Go, Ruby, Python, Node, Deno, Bun, Java, Rust);
qusp resolves and installs them all in parallel, with reproducibility locked
in `qusp.lock`. Every backend is **native Rust** — no plugin bash, no
subprocess freeloading on `rustup` / `nvm` / `pyenv`.

The pitch: **uv-grade quality, every language**, with both `uv run` and
`mise activate` styles available — pick the one that matches your workflow.

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
cargo install --git https://github.com/O6lvl4/qusp --tag v0.7.0 --bin qusp
```

### Pre-built binaries

Each release ships sha256-verified tarballs for `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`,
and `x86_64-pc-windows-msvc`. See [Releases].

[Releases]: https://github.com/O6lvl4/qusp/releases

## 30-second quickstart

```bash
qusp init                            # writes a starter qusp.toml
qusp install go 1.26.2               # pin one language
qusp install rust stable             # channel resolution (rustup-compatible)
qusp install                         # install every language pinned in qusp.toml, in parallel

qusp run go test ./...               # run with the project-pinned toolchain
qusp run rustc main.rs               # rustc, cargo, rustdoc — all resolved
qusp add tool gopls                  # routed to the Go backend, sumdb-verified
qusp add tool tsc                    # routed to Node, npm `dist.integrity` verified

quspx pnpm install                   # ephemeral run via argv[0] dispatch
qusp sync --frozen                   # reproduce exactly from qusp.lock (CI mode)
qusp outdated                        # ↑ rust 1.85.0 → 1.95.0
qusp self-update                     # in-place upgrade, sha256-verified
```

## Two modes, one tool

By design, qusp supports two entry-point styles. **Default is uv-style**:
nothing about your shell changes; everything goes through `qusp run` /
`quspx`. **Opt-in is mise-style**: a `chpwd` hook injects PATH + GOROOT +
JAVA_HOME + … so `python`, `go`, `ruby` work at the bare prompt.

```bash
# uv-style (default — global shell never modified)
$ which python
/usr/bin/python
$ qusp run python --version
Python 3.13.0

# mise-style (opt-in — installed once)
$ eval "$(qusp hook --shell zsh)"  # add to ~/.zshrc
$ cd ~/projects/myapp
$ which python
~/Library/Application Support/dev.O6lvl4.qusp/python/3.13.0/bin/python
$ cd /tmp
$ which python
/usr/bin/python                       # auto-restored on cd-out
```

## Languages

| Backend | Toolchain source | Verification | Tools |
|---|---|---|---|
| **go** | go.dev official tarballs | sha256 | full `gv` registry (gopls, golangci-lint, …) |
| **ruby** | ruby-lang.org via `ruby-build` | sha256 | bundler, rake (via `rv`) |
| **python** | python-build-standalone | sha256 (SHA256SUMS file) | _via uv routing — coming v0.10_ |
| **node** | nodejs.org official | sha256 (SHASUMS256.txt) | curated: pnpm, yarn, tsc, prettier |
| **deno** | denoland/deno releases | sha256 (inner binary) | toolchain only (use deno's own `install`) |
| **bun** | oven-sh/bun releases | sha256 (SHASUMS256.txt) | toolchain only (use bun's own `install`) |
| **java** | Foojay disco API (Temurin/Corretto/Zulu/GraalVM CE) | sha256 | mvn (sha512), gradle (sha256) |
| **rust** | static.rust-lang.org (rustup CDN) | sha256 | use `cargo install` / `cargo binstall` |
| **kotlin** | JetBrains/kotlin GitHub releases | sha256 | toolchain only (Gradle drives plugins). **requires `[java]`** |

Every install **verifies a publisher-published hash** before extracting.
Java's checksums come from Foojay's normalized API; Maven's are sha512;
Node's npm tools are verified against `dist.integrity` (sha512 base64).
qusp autodetects sha256 vs sha512 by hex-length.

### Multi-vendor (Java)

```toml
[java]
version = "21"
distribution = "temurin"           # or "corretto" | "zulu" | "graalvm_community"
```

Resolution goes through Foojay disco, the same registry SDKMAN uses, so
every distribution publishes through one normalized API. qusp downloads
straight from the publisher's CDN.

## How it differs

| | mise / asdf | proto | uv (Python) | sdkman | devbox / Nix | **qusp** |
|---|---|---|---|---|---|---|
| Languages | 100+ via plugins | ~15 | 1 | JVM only | unlimited via Nix | 8 native |
| Plugin model | bash plugins | Rust | n/a | bash | derivations | none — every backend is native Rust |
| Hash verification | varies | varies | strict | sha256 | derivation | **strict, every install** |
| Subprocess freeloading | yes (system tools) | partial | none | yes | none | **none** |
| Per-vendor (Java) | plugin per vendor | n/a | n/a | curated | per-derivation | **first-class via Foojay** |
| `run` vs `shellenv` | shellenv only | shim | run only | shellenv | shell-direct | **both, opt-in shellenv** |
| Lockfile | partial | partial | yes | no | flake.lock | yes (`qusp.lock`) |
| Reproducibility | partial | partial | uv.lock | low | high | **lockfile + content-addressed store** |
| OS-lib reproducibility | × | × | × | × | ✓ | × (out of scope — use Nix) |

**qusp's lane**: deeper than mise/asdf (no plugins, native everywhere,
strict hash verification), broader than uv (every language, not just
Python), and friendlier than Nix (no derivation language). It is **not**
trying to replace Nix for OS-library reproducibility.

### Latency

`scripts/bench.sh` measures invocation cost via [hyperfine] on
macOS-13 x86_64. Both qusp and mise have go 1.26.2 installed locally;
the project pins it through each manager's manifest.

[hyperfine]: https://github.com/sharkdp/hyperfine

| Mode | Mean | User+Sys CPU |
|---|---|---|
| `qusp run go version` | **12.0 ms** | 9 ms |
| `mise exec go version` | 12.1 ms | 9 ms |
| `mise shim go version` (default activated mode) | 49.4 ms | 39 ms |

**qusp run** is statistically tied with `mise exec`. mise's **shim
mode** — the default users hit when `mise activate` is in their
rcfile — is **~4× slower** because every command goes through a
binary wrapper that re-resolves the toolchain.

qusp doesn't have a shim layer. `qusp run` resolves and execs the
toolchain binary directly; `eval "$(qusp shellenv)"` puts the
toolchain bin/ on PATH so bare `go version` from the prompt is
literally just exec'ing the qusp-managed binary.

## Architecture

- `qusp-core` — `Backend` trait, manifest, lock, orchestrator
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
- A plugin platform. The strength is curated quality across 8 languages.
- A drop-in replacement for `cargo install` / `npm install -g` / `gem install`
  / `pip install`. Tools that have peer-dep complexity are intentionally
  not in qusp's curated registries.

## Status

- **v0.7.0** ships 8 languages, init/outdated/self-update, 5-target release matrix.
- Tested on macos-13 x86_64 (manual), CI verifies macos-14 arm64 + ubuntu-latest + windows-latest builds.
- Documentation incomplete; this README is the source of truth.

## Roadmap

- **v0.8.0** — Kotlin backend with `Backend::requires` mechanism
  (the first cross-backend dependency). Scala via Coursier.
- **v0.9.0** — Python tool routing through `uv tool install`. Tool
  registry expansion across Node + Java.
- **v1.0.0** — sigstore signature verification, sbom export,
  reproducibility audit. Enough commands stable to declare an API
  freeze.
- **Later** — Nix L1/L2/L3 interop (read flake.nix → use as resolution
  source → `qusp export nix` to flake.nix).

## Contributing

This is a single-author project right now. Issues + PRs welcome on
GitHub. Architecture deviations should come with a `docs/RFC-*.md`
proposal that matches the rest of the design philosophy: native-Rust,
strict-verification, no plugin layer.

## License

MIT
