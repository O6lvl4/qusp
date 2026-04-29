# Release prep checklist

Pre-tag drill for every qusp release. Most steps are cheap; the
**default-version drift refresh** is the one that gets skipped without
ceremony, so it's first.

## 1. Default-version drift refresh (~2 min)

```bash
cargo build --release
./scripts/refresh_default_versions.py
```

Output is a 3-section report:

- **drift** — backends whose `default_version` is older than upstream `remote[0]`
- **parity** — backends already at remote[0]
- **errors** — backends whose `qusp list <lang> --remote` failed (transient or
  list_remote bug; non-fatal, leave that row alone)

For each `drift` row, **review the top-5 remote entries plus the hint**.
The script does **not** auto-bump because several backends have known
traps the human catches:

| backend | trap |
|---|---|
| python | `remote[0]` is often an alpha (`3.x.0a7`); pick highest stable patch |
| java | `remote[0]` is often the EA build (e.g. 26.0.x); prefer `(LTS)` rows (21, 25) |
| node | `remote[0]` is current major (odd = non-LTS); prefer even-major LTS |
| ruby | rv-core list returns ascending, not descending; pick from end of remote |
| dart | list_remote sometimes returns empty (transient); leave row alone |
| go | output prefix `go1.x.y` may need stripping when comparing |
| lua | 5.5 just released; default to 5.4.x latest patch unless intentional bump |
| groovy | 5.x may break Gradle <8 compat; 4.x line still maintained |

After deciding, hand-edit **both**:

```
crates/qusp-cli/src/script.rs  →  fn default_version
crates/qusp-cli/src/main.rs    →  cmd_init template (duplicated map, x2)
```

(Keeping these two in sync is unavoidable until they're refactored —
which has explicitly been deferred since both want to be `const`-able
and Rust doesn't let const fns alloc strings.)

Then:

```bash
cargo test --release
cargo build --release
./scripts/refresh_default_versions.py   # confirm parity grew
```

Commit with a "release prep" line in the message so the diff is
auditable.

## 2. Cargo.toml workspace version bump

```bash
sed -i '' 's/^version = "<old>"/version = "<new>"/' Cargo.toml
sed -i '' 's|//! qusp CLI — v<old>.|//! qusp CLI — v<new>.|' \
    crates/qusp-cli/src/main.rs
cargo build --release
```

## 3. Test gates

```bash
cargo test --release                # unit tests, all crates
bash scripts/e2e.sh --fast          # ~30s suite, must pass
```

If you have time:

```bash
bash scripts/e2e.sh                 # full multi-language suite (~15 min)
```

## 4. Roadmap doc updates

Move the corresponding `on-hold/X.md` → `done/X.md` if shipped, update
`docs/roadmap/README.md` index. For Phase 5 audit row fixes, also
update the `hospitality-parity.md` audit table (✅/🟡/❌ counts).

## 5. Tag + push

```bash
git add ...
git commit -m "v<new>: <one-line summary>

<body>"
git tag v<new>
git push origin main && git push origin v<new>
```

## 6. CI handles the rest

The `release.yml` workflow auto-builds 5-platform binaries and bumps
the Homebrew tap. Watch the run for ~3 min, then verify
`brew upgrade O6lvl4/tap/qusp` works on a clean machine (or wait for
real users to confirm).

## Why per-step gates exist

- **Drift refresh first** — caught only via dogfood in v0.28.2 (rust
  1.85.0 vs ecosystem MSRV 1.86), now formalised.
- **Hand-edit, not auto-bump** — alpha/EA/LTS choices are
  per-language judgement; auto-bumps shipped 3.15.0a7 to a Python
  user once too often elsewhere.
- **e2e fast before tag** — full e2e takes ~15 min; tag without it
  has shipped twice with broken backends in this codebase already.
- **Two version maps to update** — script.rs (`default_version`) +
  main.rs (`cmd_init`'s template) — `cargo check` won't catch the
  drift, only `qusp init --langs=X` smoke would.
