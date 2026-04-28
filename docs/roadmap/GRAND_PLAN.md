# Qusp Grand Plan

> 「**1.0 で多言語 toolchain manager の coherent な core** として勝ち切る。
>  **2.x で uv 並のホスピタリティを 18+ 言語全部に**展開する。
>  **3.x で reproducibility と nix bridge** で graduation path を作る。」

---

## Phase 1: 1.0 Completion

**ゴール: "この toolchain manager はもう信用して使っていい" という状態。**

機能を増やさず、完成の定義を固定する。

- [x] 9 言語 native backend (go / ruby / python / node / deno / bun / java / rust / kotlin)
- [x] Multi-vendor (Java: Temurin / Corretto / Zulu / GraalVM CE)
- [x] Cross-backend deps (`Backend::requires`、Kotlin → Java の検証で実証)
- [x] Hash 検証 (sha256 / sha512、全 publisher、length 自動判別)
- [x] DDD architecture: validate → plan → execute (Phase 1-3 + 3.5 完全移行)
- [x] HttpFetcher trait (LiveHttp / MockHttp、unit-test layer 確立)
- [x] orchestrator partial-success
- [x] qusp.lock + `qusp sync --frozen`
- [x] uv-style + mise-style 両対応 (`qusp run` / `qusp shellenv` + `qusp hook`)
- [x] init / outdated / self-update
- [x] e2e suite (全 backend × isolated $HOME)
- [x] Release infra (matrix CI + Homebrew tap auto-bump + nightly e2e)
- [x] Benchmark vs mise (mise shim mode の **4× 高速** を実数で示せる)
- [x] README + ARCHITECTURE.md
- [ ] **[Daily dogfood + 1.0 release](active/dogfood-and-1.0.md)** ← active

**完成定義:** 著者本人が mise を解除して qusp daily driver で生きていける。
papercut が出尽くした時点で 1.0.0 タグ。

## Phase 2: Production Trust (1.x)

配布の信用を強化する。

- [ ] **[Sigstore signature verification](on-hold/sigstore-verification.md)** ─ sha 検証 (publisher CDN 同経路) を超えて SLSA/sigstore で独立署名チェック。
- [ ] **[Range version specs](on-hold/range-version-specs.md)** ─ `^21.0`, `~1.85.0`。
- [ ] **[`qusp upgrade`](on-hold/qusp-upgrade.md)** ─ outdated → manifest bump → sync の actionable 版。
- [ ] **[Linux benchmark](on-hold/linux-benchmark.md)** ─ macOS の数値だけ。CI nightly で永続化。
- [ ] **[Backend unit tests for remaining backends](on-hold/backend-unit-tests.md)**
- [ ] **[`qusp plan`](on-hold/qusp-plan.md)** ─ DDD `plan_*` をユーザに見せる terraform-plan 相当。dogfood で需要が出るかで決める。

## Phase 3: Tool Economy (2.x)

curated tool registry を広げる。**Phase 5 (Hospitality) の cross-language tool registry に内包される予定**。

- [ ] **[Python tools via uv routing](on-hold/python-tools-via-uv.md)** ─ Phase 5 の前提。
- [ ] **[Tool registry expansion](on-hold/tool-registry-expansion.md)** ─ Node / Java / Go の curated set 拡大。
- [ ] **[Cargo binstall integration](on-hold/cargo-binstall.md)** ─ Rust ecosystem の prebuilt route。

## Phase 4: Language Breadth (1.x → 2.x)

「ある」と言いたい言語を、coherent に増やす。**プラグイン化はしない**。

### Done (本セッション 9 連発で 18 言語に到達)

- [x] **[Zig](done/zig.md)** (v0.15.0) ─ Phase 4 第一弾、prebuilt zip + sha256
- [x] **[Julia](done/julia.md)** (v0.16.0) ─ julialang-s3 versions.json
- [x] **[Crystal](done/crystal.md)** (v0.17.0) ─ GitHub `asset.digest` で sha256 verify (sidecar 不在対応)
- [x] **[Groovy](done/groovy.md)** (v0.18.0) ─ Apache zip、`requires=["java"]`、Application Support space trap 1 度目
- [x] **[Dart](done/dart.md)** (v0.19.0) ─ Google Cloud Storage、BSD-style sha256sum
- [x] **[Scala 3](done/scala.md)** (v0.20.0) ─ direct GitHub release、Coursier wrap 不要に
- [x] **[Clojure](done/clojure.md)** (v0.21.0) ─ posix-install.sh を Rust で再実装、Application Support 空間 trap 2 度目
- [x] **[Lua](done/lua.md)** (v0.22.0) ─ source-build pattern 初投入 (PHP/R/Erlang/OCaml の雛形)、Application Support 3 度目
- [x] **[Haskell](done/haskell.md)** (v0.23.0) ─ ghcup wrap pattern 初投入、Application Support 4 度目 (autoconf 系) → no-space store

### 残り (Phase 4 完了まで)

- [ ] **[OCaml](on-hold/ocaml.md)** ─ opam-wrap pattern (Haskell の no-space store を流用予定)
- [ ] **[Elixir + Erlang](on-hold/elixir-and-erlang.md)** ─ 複合、`requires=["erlang"]`、OTP source build
- [ ] **[PHP](on-hold/php.md)** ─ source build、extension 地獄
- [ ] **[R](on-hold/r.md)** ─ OS deps 重い、source build
- [ ] **[Swift (server-side)](on-hold/swift.md)** ─ swift.org tarball、PGP sig (新検証経路)
- [ ] **[Flutter](on-hold/flutter.md)** ─ ~700MB、Dart は v0.19.0 で先行出荷済
- [ ] **[LuaJIT](on-hold/luajit.md)** ─ Lua 5.1 互換だが処理系として完全別物

### 設計知見 (本セッションで得たもの)

- **macOS Application Support space trap は 4 ケース** (Groovy launcher / Clojure 自前 sed / Lua Makefile / Haskell autoconf) で踏まれた。3 階層の対処パターンが立った: launcher patch / Makefile staging / 上流 no-space store。`memory: project_qusp_macos_appsupport_space_trap.md` 参照。
- **直接 GitHub release の `.sha256` sidecar は予想以上に普及してる**。Coursier-wrap 案は Scala/Clojure 両方で不要に。Phase 4 第二段階 (Phase 5 hospitality 並行) で「wrap が本当に必要な言語」と「直接 download で済む言語」の境界が見えてきた。
- **bootstrap-installer wrap (ghcup) は OCaml/opam で再利用可能**。

## Phase 5: Hospitality Parity (2.x)

> **新方針 (v0.24.0 起点):** 「mise/asdf より厳しい / uv より広い」のうち
> 「広い」軸を立て、**uv が Python 単体に対してやってる ergonomic 密度を 18+ 言語横断で再現する**。

position 設計の根拠は **[`hospitality-parity.md`](on-hold/hospitality-parity.md)** 参照。

### Done

- [x] **[`qusp x <script>` extension-routing](done/x-script-routing.md)** (v0.24.0) ─ 16 言語で `qusp x ./hello.<ext>` が fresh machine で auto install + exec する uv 級体験を立てた。

### 残り

- [ ] **[Did-you-mean fuzzy: 全 backend 展開](on-hold/did-you-mean-cross-backend.md)** ─ Python だけにある fuzzy を 18 backend で。
- [ ] **[Progress display を uv 級に揃える](on-hold/progress-display-uv-class.md)** ─ spinner / ETA / "downloaded N of M" 統一。
- [ ] **[Cross-language tool install registry](on-hold/tool-registry-cross-language.md)** ─ `qusp tool install ruff/gopls/scalafmt/...` を 1 動詞で。Phase 3 を内包。
- [ ] **[Inline script metadata (PEP 723 風)](on-hold/inline-script-metadata.md)** ─ `# qusp: lua = 5.4.7` で auto pin。
- [ ] **[Error richness: distribution defaults](on-hold/error-richness-distribution-defaults.md)** ─ uv 級 actionable error。
- [ ] **[shellenv auto-eval](on-hold/shellenv-auto-eval.md)** ─ rcfile 編集を要求しない経路 (現状 D 案 = no-op で行く前提)。

## Phase 6: Reproducibility & Nix Bridge (3.x)

「qusp.lock があれば手元の installed と同一の bit を得られる」を保証する。

(旧 Phase 5。Hospitality を 2.x の主軸に上げたので 3.x へ後ろ倒し。)

- [ ] **[SBOM export](on-hold/sbom-export.md)**
- [ ] **[Reproducibility audit](on-hold/reproducibility-audit.md)**
- [ ] **[Nix L1: detect substitutes](on-hold/nix-l1.md)**
- [ ] **[Nix L2: read flake.nix](on-hold/nix-l2.md)**
- [ ] **[Nix L3: export nix](on-hold/nix-l3.md)**

---

## 設計原則 (Phase 全体で不変)

1. **Native Rust everywhere.** プラグイン化しない。全 backend がこの repo の中。
2. **Hash 検証必須。** `--insecure` フラグは作らない。publisher が hash を出さない場合は backend を作らない (curated SHA table か bootstrap-installer wrap で代替)。
3. **uv-style デフォルト + mise-style opt-in.** shellenv は opt-in。
4. **No subprocess freeloading.** 例外は明示: ruby-build / go install / Lua make / ghcup install / opam (将来) は spawn_blocking で囲い、各 backend doc で justification を残す。
5. **DDD: validate → plan → execute** の 3 層を崩さない。新 backend は `Backend` trait の実装一本、effect (HTTP / progress) は trait 経由で注入。
6. **One coherent core, no plugins.** 言語追加は repo に PR、qusp-plugin-* は無い。
7. **Hospitality first (新):** v0.24.0 以降、新 backend は extension-routing と inline metadata の対応も同時に shipping する。Phase 5 が settle すれば、新 backend 追加コストは「runtime install + extension mapping + default version」の 3 点だけになる予定。
