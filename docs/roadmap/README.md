# Qusp Roadmap

> [GRAND_PLAN.md](GRAND_PLAN.md) — 6 フェーズの全体戦略 (Phase 5 = Hospitality Parity を v0.24.0 起点で新設)

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
| [Linux benchmark](on-hold/linux-benchmark.md) | 今は macOS のみ。CI nightly で永続化 | Phase 2 |
| [Backend unit tests](on-hold/backend-unit-tests.md) | python/rust 以外の backends | Phase 2 |
| [`qusp plan`](on-hold/qusp-plan.md) | terraform-plan 相当の dry-run。dogfood で需要が出れば | Phase 2 |

## On Hold — Phase 3: Tool Economy (2.x)

> **Phase 5 (Hospitality) の cross-language tool registry に内包される予定。**

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [Python tools via uv routing](on-hold/python-tools-via-uv.md) | `qusp add tool ruff` → uv tool install | Phase 3 |
| [Tool registry expansion](on-hold/tool-registry-expansion.md) | Node / Java / Go の curated set 拡大 | Phase 3 |
| [Cargo binstall integration](on-hold/cargo-binstall.md) | Rust ecosystem の prebuilt パス | Phase 3 |

## On Hold — Phase 4: Language Breadth (1.x → 2.x)

> 本セッション (v0.15.0–v0.23.0) で **9 言語連続出荷、合計 18 backend** に到達。
> 残り 7 言語 + LuaJIT で Phase 4 完了。

### Source-build 系 (難易度: 中-高)

| 項目 | コメント |
|---|---|
| [LuaJIT](on-hold/luajit.md) | Lua 5.1 互換だが完全に別物。Lua 本体は v0.22.0 で先行出荷済 |
| [PHP](on-hold/php.md) | source build、extension 地獄 |
| [R](on-hold/r.md) | OS deps 重い、source build |
| [Swift (server-side)](on-hold/swift.md) | Linux distro 別 tarball、PGP sig (新検証経路) |
| [Elixir + Erlang](on-hold/elixir-and-erlang.md) | Erlang OTP source build、Elixir prebuilt zip。`requires = ["erlang"]` |

### Bootstrap-installer wrap 系

| 項目 | bootstrap | コメント |
|---|---|---|
| [OCaml](on-hold/ocaml.md) | opam | Haskell の no-space store パターンを流用予定 |

### Single-binary 系

| 項目 | 入手経路 | コメント |
|---|---|---|
| [Flutter](on-hold/flutter.md) | storage.googleapis.com | SDK ~700MB、Dart は v0.19.0 で先行出荷済 |

## On Hold — Phase 5: Hospitality Parity (2.x) ★ 新

> **新方針:** uv が Python 単体に対してやってる ergonomic 密度を 18+ 言語横断で再現する。
> 設計の根拠と **実測 audit (29 項目, 2026-04-28 snapshot, ✅ 12 / 🟡 11 / ❌ 9 / 設計差 3 / scope 違い 2 / バグ 1) + 完了基準 (✅ 18+/23 評価対象 ≥ 78%)** は **[hospitality-parity.md](on-hold/hospitality-parity.md)** に記録。

### Umbrella

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [Hospitality 概念 + 実測 audit + 完了基準](on-hold/hospitality-parity.md) | umbrella doc, 全 sub-task の親 | Phase 5 |

### High priority (audit ❌ 解決)

| 項目 | audit row | 説明 |
|---|---|---|
| [Did-you-mean fuzzy 全 backend 展開](on-hold/did-you-mean-cross-backend.md) | A5 / A5b / K1 | Python 専用の fuzzy を全 18 backend で + silent substitution 通告 |
| [Progress display を uv 級に揃える](on-hold/progress-display-uv-class.md) | A3 / A4 | "Downloading X (NMiB)" 単行 progress、build subprocess も統一 |
| [Cross-language tool install registry](on-hold/tool-registry-cross-language.md) | D3 / D4 | `qusp tool install ruff/gopls/scalafmt/...` (uvx 同形) |
| [Inline script metadata (PEP 723 風)](on-hold/inline-script-metadata.md) | D2 | `# qusp: lua = 5.4.7` で auto pin |
| [Network resilience + retry](on-hold/network-resilience-retry.md) | A7 | HttpFetcher に retry layer、parse error 露出を消す |
| [Cache management](on-hold/cache-management.md) | G2 / G3 | `qusp cache clean / prune --ci`、reachable-set GC |
| [Force reinstall flag](on-hold/force-reinstall.md) | N1 | `qusp install --reinstall` (`-r`, `-f`) |
| [`--help` richness + pager](on-hold/help-richness-and-pager.md) | Q1 / Q2 / Z1 | clap long help + `[env: QUSP_*=]` + `--no-pager` |
| [Machine-readable JSON output](on-hold/machine-readable-output.md) | R1 | `--output-format json` 全 list / current / doctor 系で |
| [Verbose `-v` + env var contract](on-hold/verbose-flag.md) | Y1 / S1 | `tracing` 導入 + `QUSP_*` env vars exposed in `--help` |

### Medium priority (audit 🟡 → ✅)

| 項目 | audit row | 説明 |
|---|---|---|
| [Error richness: distribution defaults](on-hold/error-richness-distribution-defaults.md) | A5 / K1 / K2 | InstallErr enum + 各 error に actionable next-step |
| [List remote richness](on-hold/list-remote-richness.md) | B1 / I1 | impl tag + install-status + source col |
| [Resolve current の絶対 path](on-hold/current-resolved-path.md) | B3 | `qusp current python --resolved` |
| [version 文字列に build rev + date](on-hold/version-build-metadata.md) | C2 | build.rs で git rev / build date 埋め込み |
| [PATH not-on-path 案内](on-hold/path-not-on-path-guidance.md) | D6 | install 後 hint + `qusp doctor` shell-hook 検出 |

### Low priority / Defer

| 項目 | audit row | 説明 |
|---|---|---|
| [shellenv auto-eval](on-hold/shellenv-auto-eval.md) | D6 延長 | rcfile 編集ゼロ経路 (現状 D 案 = no-op で行く前提) |

### Phase 5 外 (correctness バグ、independent fix)

| 項目 | audit row | 説明 |
|---|---|---|
| [Concurrent install safety (file lock)](on-hold/concurrent-install-safety.md) | W1 | 並列 `qusp install` の race 防止、`fs2::FileExt` ベース |
| (F4 lockfile on failed install) | F4 | A7 と同じ install-path hardening、独立 doc 不要 |

## On Hold — Phase 6: Reproducibility & Nix Bridge (3.x)

(旧 Phase 5。Hospitality を 2.x の主軸に上げたので 3.x へ後ろ倒し。)

| 項目 | 説明 | Grand Plan |
|---|---|---|
| [SBOM export](on-hold/sbom-export.md) | `qusp sbom` (SPDX/CycloneDX) | Phase 6 |
| [Reproducibility audit](on-hold/reproducibility-audit.md) | `qusp verify` で手元 vs lock | Phase 6 |
| [Nix L1: detect substitutes](on-hold/nix-l1.md) | `/nix/store` 既在を使い回す | Phase 6 |
| [Nix L2: read flake.nix](on-hold/nix-l2.md) | flake.nix を resolution source に | Phase 6 |
| [Nix L3: export nix](on-hold/nix-l3.md) | `qusp.toml` + `qusp.lock` → flake.nix | Phase 6 |

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
| [Zig backend (Phase 4 第一弾)](done/zig.md) | v0.15.0 | Phase 4 |
| [Julia backend](done/julia.md) | v0.16.0 | Phase 4 |
| [Crystal backend](done/crystal.md) | v0.17.0 | Phase 4 |
| [Groovy backend](done/groovy.md) | v0.18.0 | Phase 4 |
| [Dart backend](done/dart.md) | v0.19.0 | Phase 4 |
| [Scala 3 backend](done/scala.md) | v0.20.0 | Phase 4 |
| [Clojure backend](done/clojure.md) | v0.21.0 | Phase 4 |
| [Lua backend (source-build pattern 初投入)](done/lua.md) | v0.22.0 | Phase 4 |
| [Haskell backend (ghcup wrap pattern 初投入)](done/haskell.md) | v0.23.0 | Phase 4 |
| [`qusp x <script>` extension-routing (Phase 5 第一弾)](done/x-script-routing.md) | v0.24.0 | Phase 5 |
| [e2e test scenarios](done/e2e-tests.md) | scripts/e2e/* | Phase 1 |
| [Benchmark vs mise](done/benchmark-vs-mise.md) — shim mode 4× 速い実数 | scripts/bench.sh | Phase 1 |

---

## 凡例

- `active/` — 進行中
- `on-hold/` — スコープ外、要件出たら active に戻す
- `done/` — 完了。歴史記録として残す
