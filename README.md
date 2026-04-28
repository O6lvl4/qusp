# qusp

> Every language toolchain in superposition. `cd` collapses to one.

`qusp` is a multi-language toolchain manager. One `qusp.toml` describes the
Go, Ruby, Python, Terraform, Node, Java, and Deno versions a project needs;
`qusp` resolves and installs them all, in parallel, with reproducibility
locked in `qusp.lock`. Per-language deep treatment, not asdf-grade
plugin shallowness.

The name comes from Greg Egan's *Schild's Ladder*: a **qusp** is a
quantum-superposition processor that holds many possible states at once
until observation collapses them. That is exactly what a project's
toolchain feels like — many candidate versions in superposition, until you
`cd` into the project and the manifest collapses the wavefunction.

## Positioning

```
homebrew → mise/asdf → qusp → devbox → flox → nix-shell → Nix → NixOS
                       ^^^^
              uv-grade convenience for every language,
              Nix-friendly when you're ready to graduate.
```

| | mise / asdf | **qusp** | devbox | Nix |
|---|---|---|---|---|
| Multi-language | ✓ shallow plugins | ✓ deep per-lang backends | ✓ via Nix | ✓ |
| Per-lang manifest direct read (`go.mod`, `Gemfile`, `pyproject.toml`) | partial | ✓ | indirect | indirect |
| Lockfile reproducibility | partial | ✓ `qusp.lock` | yes (Nix) | yes (Nix) |
| OS lib reproducibility | × | × | ✓ | ✓ |
| Learning curve | hours | hours | days | months |
| Nix interop | none | **planned: L0/L1/L2/L3** | full | native |

## Status

🚧 Pre-alpha. v0.0.1 ships Go + Python only.

## Roadmap

- **v0.0.x** — Go (via `gv`) + Python (via `uv`) backends. `qusp.toml` /
  `qusp.lock` schemas. `qusp install`, `qusp run`, `qusp tree`,
  `qusp sync`.
- **v0.1.0** — Ruby (via `rv`), Terraform, Deno backends. `qusp init`,
  `qusp outdated`, `qusp upgrade`, `quspx` ephemeral run.
- **v0.2.0** — Node (with corepack/pnpm/yarn-as-tools), Java (Temurin
  default + Corretto/Liberica options).
- **v0.3.0 (Nix L1)** — detect `/nix/store`, reuse substitutes when a
  matching toolchain is already there.
- **v0.4.0 (Nix L2)** — read `flake.nix` declared package versions as a
  resolution source.
- **v0.5.0 (Nix L3)** — `qusp export nix` to write a `flake.nix` from
  `qusp.toml` + `qusp.lock`. The graduation path to full Nix.

## Quickstart (when shipped)

```bash
qusp init                      # writes qusp.toml
qusp add go 1.26.2             # pin a toolchain
qusp add python 3.12
qusp add ruby 3.3.5
qusp add tool gopls            # auto-routed to go backend
qusp add tool ruff             # auto-routed to python (uv tool install)
qusp sync --frozen             # CI: install exactly what's locked
qusp tree                      # full multi-language env at a glance
quspx golangci-lint run        # ephemeral, no project state touched
```

## Architecture

`qusp` is built on top of [`anyv-core`](https://github.com/O6lvl4/anyv-core)
(paths / presentation / extract / fs / argv0 / target / selfupdate) — the
same substrate used by [`gv`](https://github.com/O6lvl4/gv) and
[`rv`](https://github.com/O6lvl4/rv).

Backends are pluggable. Today: `qusp-backend-go`, `qusp-backend-python`.
Tomorrow: ruby, terraform, deno, node, java. Each backend implements a
trait that the orchestration core calls; the CLI doesn't know what
language it's talking to.

See [docs/DESIGN.md](docs/DESIGN.md) for the trait surface, manifest
schema, and per-language semantics.

## Why `qusp` (not yet another `*v`)

Because gv + rv + uv + jv + vv + … is namespace pollution. `qusp` is
one tool, one binary, one manifest, every language. The single-letter
prefix family stays useful (and gv/rv aren't deprecated — they remain as
focused tools), but the casual every-day-multi-language workflow lives
under `qusp`.

## License

MIT
