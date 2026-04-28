# qusp architecture

This document explains how qusp is wired together so you can reason about
new backends, debug existing ones, or rip out parts safely.

## Crates

```
qusp/
├── crates/
│   ├── qusp-core/          # library: traits, types, orchestration logic
│   └── qusp-cli/           # binary: command surface
├── packaging/
│   └── homebrew/           # tap formula template, rendered at release time
└── .github/workflows/      # CI matrix + tag-triggered release
```

Direct dependencies:

| Crate | What it gives qusp |
|---|---|
| [`anyv-core`](https://github.com/O6lvl4/anyv-core) | XDG paths, archive extraction (tar.gz, zip), argv[0] dispatch, target triple, presentation (spinner, color, `say!`), self-update primitives |
| [`gv-core`](https://github.com/O6lvl4/gv) | Go toolchain install (go.dev tarballs, sumdb), Go tool registry, sumdb h1: hash verification |
| [`rv-core`](https://github.com/O6lvl4/rv) | Ruby toolchain install via ruby-build, gemspec resolution |
| `reqwest`, `tokio`, `serde`, `clap`, `sha2`, `base64`, `tar`, `flate2`, `zip`, `indicatif` | the usual cast |

`gv-core` and `rv-core` are linked as **Cargo dependencies**, not
spawned as subprocesses. Same store, same lock format, same install
result whether you call `gv install 1.26.2` or
`qusp install go 1.26.2`.

## The Backend trait

Every language is a `Backend`:

```rust
pub trait Backend: Send + Sync {
    fn id(&self) -> &'static str;
    fn manifest_files(&self) -> &[&'static str];
    fn knows_tool(&self, _name: &str) -> bool { false }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>>;
    async fn install(&self, paths: &Paths, version: &str, opts: &InstallOpts) -> Result<InstallReport>;
    fn uninstall(&self, paths: &Paths, version: &str) -> Result<()>;
    fn list_installed(&self, paths: &Paths) -> Result<Vec<String>>;
    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>>;

    async fn resolve_tool(&self, client: &reqwest::Client, name: &str, spec: &ToolSpec) -> Result<ResolvedTool>;
    async fn install_tool(&self, paths: &Paths, toolchain_version: &str, resolved: &ResolvedTool) -> Result<LockedTool>;
    fn tool_bin_path(&self, paths: &Paths, locked: &LockedTool) -> PathBuf;
    fn build_run_env(&self, paths: &Paths, version: &str, cwd: &Path) -> Result<RunEnv>;
}
```

Adding a new language: one file under `crates/qusp-core/src/backends/`
plus one `r.register(Arc::new(MyBackend))` line in `cli/main.rs::build_registry`.

`InstallOpts` carries vendor-specific knobs — currently just
`distribution: Option<String>` for Java. Single-source backends ignore.

## The orchestrator

The CLI never talks to a specific backend. It builds a `BackendRegistry`,
constructs an `Orchestrator { registry, paths }`, and calls high-level
methods:

```rust
orch.install_toolchains(&manifest)        // parallel install of every pinned lang
orch.install_tools(&manifest, &mut lock, frozen, &client)
orch.prune_stale_tools(&manifest, &mut lock)
orch.sync(&manifest, &mut lock, frozen, &client)
orch.route_tool("gopls")                  // → ("go", Arc<GoBackend>)
orch.add_tool(&mut manifest, &mut lock, "gopls", "latest", &client)
orch.find_tool(&lock, "gopls")            // → (lang, locked, bin_path)
orch.build_run_env(&lock, cwd, prefer_lang)  // merges PATH/GOROOT/GEM_HOME/...
```

`install_toolchains` and `install_tools` use `futures::try_join_all`
for true parallelism — installing 8 languages takes the time of the
slowest, not the sum.

`route_tool` is sync (no network) thanks to each backend's static
`knows_tool(&str)` registry. `qusp add tool gopls` doesn't need to
guess which backend gopls belongs to.

## On-disk layout

```
~/Library/Application Support/dev.O6lvl4.qusp/        # macOS data dir
├── store/<sha-prefix>/                                # content-addressed installs
│   ├── 4d2b8c.../jdk-21.0.11+10/Contents/Home/...
│   └── ...
├── go/<version>          → symlink to store/...
├── ruby/<version>        → symlink to store/...
├── python/<version>      → symlink to store/...
├── node/<version>        → symlink to store/...
├── deno/<version>        → symlink to store/...
├── bun/<version>         → symlink to store/...
├── java/<distribution>-<version>  → symlink to JAVA_HOME inside store/...
├── rust/<version>        → symlink to store/.../merged/
├── node-tools/<package>/<v>/<sha>/package/...
├── java-tools/<package>/<v>/<sha>/...
└── ...
```

Every install is **content-addressed by sha-prefix**. Installing the
same toolchain twice from different versions of qusp is a no-op (the
sha-prefixed dir already exists). Switching version pin is a symlink
flip — no re-download.

## qusp.toml + qusp.lock

```toml
# qusp.toml
[go]
version = "1.26.2"

[java]
version = "21"
distribution = "temurin"

[node]
version = "22.9.0"

[node.tools]
pnpm = "latest"
tsc = "latest"
```

```toml
# qusp.lock — written by `qusp sync` and `qusp add tool`
version = 1

[go]
version = "1.26.2"
upstream_hash = ""

[java]
version = "21"
distribution = "temurin"
upstream_hash = ""

[node]
version = "22.9.0"
upstream_hash = ""

[[node.tools]]
name = "pnpm"
package = "pnpm"
version = "10.33.2"
bin = "/.../qusp/node-tools/pnpm/10.33.2/<sha>/package/bin/pnpm.cjs"
upstream_hash = "sha512-..."   # npm dist.integrity verbatim
built_with = "22.9.0"
```

`qusp sync --frozen` uses lock as truth: refuses any `resolve_tool`
calls, refuses to bump the lock. Designed for CI.

## `qusp run` vs `qusp shellenv`

Both modes go through the same `Orchestrator::build_run_env`. The
difference is *who* applies the env:

- `qusp run <cmd>` — qusp itself sets PATH/JAVA_HOME/etc as the
  child process's env. The shell is untouched.
- `qusp shellenv` — qusp prints `export …` lines for the shell to
  source. `qusp hook` wraps this in a chpwd handler so `cd` triggers
  re-application; on cd-out the previously-captured `_QUSP_PATH_BASELINE`
  is restored and `_QUSP_LAST_KEYS` are unset.

Both are first-class. The lock-as-truth contract is identical.

## Verification chain

| Backend | Source | Hash | Verified against |
|---|---|---|---|
| go | go.dev | sha256 | go.dev's per-asset `.sha256` sidecar |
| ruby | ruby-lang.org | sha256 | ruby-build's recipe |
| python | python-build-standalone | sha256 | release `SHA256SUMS` file |
| node | nodejs.org | sha256 | release `SHASUMS256.txt` |
| deno | denoland/deno | sha256 (inner binary) | per-asset `.sha256sum` |
| bun | oven-sh/bun | sha256 | release `SHASUMS256.txt` |
| java | Foojay → Adoptium/Amazon/Azul/Oracle | sha256 | Foojay-resolved or `checksum_uri` |
| rust | static.rust-lang.org | sha256 | per-asset `.sha256` |
| Go tools | proxy.golang.org | sumdb h1: | go.sum-style |
| npm tools | registry.npmjs.org | sha512 (base64) | npm `dist.integrity` |
| Maven | archive.apache.org | sha512 | per-asset `.sha512` |
| Gradle | services.gradle.org | sha256 | per-asset `.sha256` |

Mismatches refuse the install. There is no `--insecure` flag.

## Process model

qusp is a single binary. `quspx` is the same binary dispatched via
argv[0] (a symlink in `bin/`). `qusp run` `execve`s child processes
directly — there is no daemon, no shim wrapper, no IPC.

PATH injection (`shellenv` mode) does not use shims. The user's
`python` resolves to qusp's via standard PATH lookup, which is
structurally faster than mise/asdf shim resolution because there's no
extra exec hop.

## Adding a new backend (cheat sheet)

```rust
// crates/qusp-core/src/backends/<lang>.rs
pub struct MyLangBackend;

#[async_trait]
impl Backend for MyLangBackend {
    fn id(&self) -> &'static str { "mylang" }
    fn manifest_files(&self) -> &[&'static str] { &[".mylang-version"] }
    fn knows_tool(&self, name: &str) -> bool { /* curated set */ }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> { /* walk for .mylang-version */ }
    async fn install(&self, paths: &Paths, v: &str, _: &InstallOpts) -> Result<InstallReport> {
        // 1. Resolve asset URL + checksum
        // 2. Download bytes (reqwest)
        // 3. Verify sha256 (sha2::Sha256)
        // 4. Extract via anyv_core::extract::extract_archive
        // 5. Symlink versions/<lang>/<v> at the install dir
    }
    // ...
}
```

```rust
// crates/qusp-cli/src/main.rs::build_registry
r.register(Arc::new(backends::mylang::MyLangBackend));
```

```rust
// crates/qusp-core/src/backends/mod.rs
pub mod mylang;
```

That's it. The orchestrator, CLI, lock format, shellenv, hook, and
release infra all pick up the new backend automatically.

## Things qusp deliberately does *not* do

- **Plugin layer.** Every backend is in this repo, written in Rust,
  reviewed by the same hand. mise/asdf chose breadth via plugin
  ecosystems; qusp chose curated depth.
- **Build from source.** Except where it's the only option (Ruby via
  ruby-build), qusp downloads pre-built binaries the publisher already
  released. Reproducibility comes from sha verification, not from
  rebuilding.
- **Library/dependency management.** `cargo` / `npm` / `pip` / `Maven`
  do that better. qusp manages the *toolchain* that those run on.
- **Cross-language artifacts.** A "tool" in qusp lives entirely under
  one backend. A tool that requires Python *and* Node is the user's
  problem — pin both.
- **Auto-detection of `*-version` files for activation.** qusp.toml is
  the source of truth in a project. `.python-version` etc are read
  only by `qusp current` for informational purposes, not for activation.
