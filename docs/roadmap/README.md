# Qusp Roadmap

> [GRAND_PLAN.md](GRAND_PLAN.md) — 5 フェーズの全体戦略

## Active

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [Daily dogfood + 1.0](active/dogfood-and-1.0.md) | mise を外して qusp daily driver、papercut 拾いきって v1.0.0 | Phase 1 |

## On Hold — Phase 2: Production Trust (1.x)

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [Sigstore signature verification](on-hold/sigstore-verification.md) | sha verification を超えた SLSA / sigstore | Phase 2 |
| [Range version specs](on-hold/range-version-specs.md) | `^21.0`, `~1.85.0` | Phase 2 |
| [`qusp upgrade`](on-hold/qusp-upgrade.md) | outdated → manifest bump → sync | Phase 2 |
| [Linux benchmark](on-hold/linux-benchmark.md) | 今は macOS のみ。CI nightly で permanenent | Phase 2 |
| [Backend unit tests](on-hold/backend-unit-tests.md) | python/rust 以外の 7 backends | Phase 2 |
| [`qusp plan`](on-hold/qusp-plan.md) | terraform-plan 相当の dry-run。dogfood で需要が出れば | Phase 2 |

## On Hold — Phase 3: Tool Economy (2.x)

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [Python tools via uv routing](on-hold/python-tools-via-uv.md) | `qusp add tool ruff` → uv tool install | Phase 3 |
| [Tool registry expansion](on-hold/tool-registry-expansion.md) | Node / Java curated set を倍 | Phase 3 |
| [Cargo binstall integration](on-hold/cargo-binstall.md) | Rust ecosystem の prebuilt パス | Phase 3 |

## On Hold — Phase 4: Language Breadth (2.x+)

| 項目 | 説明 |
|---|---|
| [JVM family via Coursier](on-hold/jvm-family-via-coursier.md) | Scala / Clojure / Groovy |
| [Elixir + Erlang](on-hold/elixir-and-erlang.md) | `requires = ["erlang"]` の入れ子 |
| [Lua / LuaJIT](on-hold/lua.md) | |
| [PHP](on-hold/php.md) | extension の扱いが難所 |
| [Dart / Flutter](on-hold/dart-and-flutter.md) | Flutter SDK の重さ |
| [Swift (server-side)](on-hold/swift.md) | swift.org tarball 直 |

## On Hold — Phase 5: Reproducibility & Nix Bridge (3.x)

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [SBOM export](on-hold/sbom-export.md) | `qusp sbom` (SPDX/CycloneDX) | Phase 5 |
| [Reproducibility audit](on-hold/reproducibility-audit.md) | `qusp verify` で手元 vs lock | Phase 5 |
| [Nix L1: detect substitutes](on-hold/nix-l1.md) | `/nix/store` 既在を使い回す | Phase 5 |
| [Nix L2: read flake.nix](on-hold/nix-l2.md) | flake.nix を resolution source に | Phase 5 |
| [Nix L3: export nix](on-hold/nix-l3.md) | `qusp.toml` + `qusp.lock` → flake.nix | Phase 5 |

## Done

直近で済んだもの。`done/<name>.md` 詳細あり。

| 項目 | 出荷 | Grand Plan |
|---|---|---|
| [Native backends — 6 langs initial](done/initial-six-backends.md) | v0.4.0 | Phase 1 |
| [Java + multi-vendor (Foojay)](done/java-foojay.md) | v0.5.0 | Phase 1 |
| [Cross-backend `requires`](done/cross-backend-requires.md) | v0.5.0 → v0.9.0 | Phase 1 |
| [Release infra](done/release-infra.md) — CI matrix + Homebrew tap | v0.6.0 | Phase 1 |
| [Rust + Bun](done/rust-and-bun.md) | v0.7.0 | Phase 1 |
| [Documentation](done/documentation.md) — README + ARCHITECTURE | v0.8.0 | Phase 1 |
| [Python fuzzy match + partial-success install](done/python-fuzzy-and-partial.md) | v0.8.1 | Phase 1 |
| [Kotlin (cross-backend dep の実証)](done/kotlin.md) | v0.9.0 | Phase 1 |
| [DDD Phase 1: PinnedManifest](done/ddd-phase-1-pinned-manifest.md) | v0.10.0 | Phase 1 |
| [DDD Phase 2: pure plan / typed errors](done/ddd-phase-2-plan.md) | v0.11.0 | Phase 1 |
| [DDD Phase 3: HttpFetcher trait + Mock](done/ddd-phase-3-effects.md) | v0.12.0 | Phase 1 |
| [DDD Phase 3.5: backend body migration](done/ddd-phase-3-5-backend-migration.md) | v0.12.1 → v0.13.0 | Phase 1 |
| [Audit-driven full migration completion](done/full-migration-completion.md) | v0.14.0 | Phase 1 |
| [e2e test scenarios](done/e2e-tests.md) | scripts/e2e/* | Phase 1 |
| [Benchmark vs mise](done/benchmark-vs-mise.md) — shim mode 4× 速い実数 | scripts/bench.sh | Phase 1 |

---

## 凡例

- `active/` — 進行中
- `on-hold/` — スコープ外、要件出たら active に戻す
- `done/` — 完了。歴史記録として残す
