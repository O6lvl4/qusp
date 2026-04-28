# Qusp Grand Plan

> 「1.0 で **多言語 toolchain manager の coherent な core** として勝ち切る。
>  機能を増やさず、shipped と claim の整合を固める。
>  v2.x 以降に "tool 配備の経済圏" を作る。」

---

## Phase 1: 1.0 Completion ← **NOW**

**ゴール: "この toolchain manager はもう信用して使っていい" という状態。**

機能を増やさず、完成の定義を固定する。

- [x] 9 言語 native backend (go / ruby / python / node / deno / bun / java / rust / kotlin)
- [x] Multi-vendor (Java: Temurin / Corretto / Zulu / GraalVM CE)
- [x] Cross-backend deps (`Backend::requires`, Kotlin → Java の検証で実証)
- [x] Hash 検証 (sha256 / sha512、全 publisher、length 自動判別)
- [x] DDD architecture: validate → plan → execute (Phase 1-3 + 3.5 完全移行)
- [x] HttpFetcher trait (LiveHttp / MockHttp、unit-test layer 確立)
- [x] orchestrator partial-success (1 backend 失敗で他を巻き込まない)
- [x] qusp.lock + `qusp sync --frozen`
- [x] uv-style + mise-style 両対応 (`qusp run` / `qusp shellenv` + `qusp hook`)
- [x] init / outdated / self-update
- [x] e2e suite (9 backends × isolated $HOME)
- [x] Release infra (matrix CI + Homebrew tap auto-bump + nightly e2e)
- [x] Benchmark vs mise (mise shim mode の **4× 高速**を実数で示せる)
- [x] README + ARCHITECTURE.md
- [ ] **[Daily dogfood + 1.0 release](active/dogfood-and-1.0.md)** ← active

**完成定義:** 著者本人が mise を解除して qusp daily driver で生きていける。
papercut が出尽くした時点で 1.0.0 タグ。

## Phase 2: Production Trust (1.x)

配布の信用を強化する。手のひら返しが起きないことを保証する。

- [ ] **[Sigstore signature verification](on-hold/sigstore-verification.md)** — sha 検証は publisher CDN と同じ経路で来る = 侵害耐性が無い。SLSA / sigstore で配布物に独立署名チェック。
- [ ] **[Range version specs](on-hold/range-version-specs.md)** — `^21.0`, `~1.85.0`。今は exact pin only。
- [ ] **[`qusp upgrade`](on-hold/qusp-upgrade.md)** — `outdated` の actionable 版。manifest を bump して sync。
- [ ] **[Linux benchmark](on-hold/linux-benchmark.md)** — 今は macOS-13 x86_64 の数値だけ。Linux で別ナンバー、CI nightly で永続化。
- [ ] **[Backend unit tests for remaining backends](on-hold/backend-unit-tests.md)** — python と rust だけ unit-test がある。残り 7 backends の URL 構築・パース層に MockHttp ベースのテスト。
- [ ] **[`qusp plan`](on-hold/qusp-plan.md)** — DDD Phase 2 で作った純粋 `plan_sync` をユーザーに見せる terraform-plan 相当。**dogfood で需要が出るかで決める**。

## Phase 3: Tool Economy (2.x)

curated tool registry を広げる。「ユーザーが `[<lang>.tools]` に書ける固有名詞」を増やす。

- [ ] **[Python tools via uv routing](on-hold/python-tools-via-uv.md)** — `qusp add tool ruff` を uv tool install に dispatch。Python の唯一空白を埋める。
- [ ] **[Tool registry expansion](on-hold/tool-registry-expansion.md)** — Node / Java の curated set を倍。Go の gv-core 経由 registry もより広く。
- [ ] **[Cargo binstall integration](on-hold/cargo-binstall.md)** — Rust ecosystem は `cargo install` が事実上の正解だが、prebuilt が欲しい。`qusp add tool ripgrep` を cargo-binstall に dispatch。

## Phase 4: Language Breadth (2.x+)

「ある」と言いたい言語を、coherent に増やす。プラグイン化はしない。

- [ ] **[Scala / Clojure / Groovy via Coursier](on-hold/jvm-family-via-coursier.md)** — `Backend::requires = ["java"]` の第二の使用例。Coursier 自体が JVM-bootstrapping なので入れ子。
- [ ] **[Elixir (+ Erlang dep)](on-hold/elixir-and-erlang.md)** — `Backend::requires = ["erlang"]` で elixir を組む。erlang は OTP のソースビルドが避けがたい (kerl 相当)。
- [ ] **[Lua / LuaJIT](on-hold/lua.md)** — シンプル、CLI 実装系で需要、prebuilt が薄いので注意。
- [ ] **[PHP](on-hold/php.md)** — 2026 でも production 大量。難所は extension。
- [ ] **[Dart / Flutter](on-hold/dart-and-flutter.md)** — Flutter SDK の重さをどう扱うか。
- [ ] **[Swift (server-side)](on-hold/swift.md)** — swift.org の tarball 直、Linux/macOS 双方。

## Phase 5: Reproducibility & Nix Bridge (3.x)

「qusp.lock があれば手元の installed と同一の bit を得られる」を保証する。

- [ ] **[SBOM export](on-hold/sbom-export.md)** — `qusp sbom` で SPDX/CycloneDX 出力。
- [ ] **[Reproducibility audit](on-hold/reproducibility-audit.md)** — `qusp verify` で手元の install hash と lock の upstream_hash を突合。
- [ ] **[Nix L1: detect substitutes](on-hold/nix-l1.md)** — `/nix/store` を見て、既に入ってるなら使い回す。
- [ ] **[Nix L2: read flake.nix](on-hold/nix-l2.md)** — flake.nix の package 宣言を resolution source として読む。
- [ ] **[Nix L3: export nix](on-hold/nix-l3.md)** — `qusp export nix` で `qusp.toml` + `qusp.lock` から flake.nix を生成。Nix への graduation path。

---

## 設計原則 (Phase 全体で不変)

1. **Native Rust everywhere.** プラグイン化しない。9 言語全部この repo の中。
2. **Hash 検証必須。** `--insecure` フラグは作らない。publisher が hash を出さない場合は backend を作らない。
3. **uv-style デフォルト + mise-style opt-in.** shellenv は opt-in。
4. **No subprocess freeloading.** gv-core / rv-core は Cargo 依存、reqwest 経由で publisher CDN 直接、ruby-build と go install は spawn_blocking で唯一の例外 (各 backend の README で明示)。
5. **DDD: validate → plan → execute** の 3 層を崩さない。新 backend は `Backend` trait の実装一本、effect (HTTP) は trait 経由で注入。
6. **One coherent core, no plugins.** 言語追加は repo に PR、qusp-plugin-* は無い。
