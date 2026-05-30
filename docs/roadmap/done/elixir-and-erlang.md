# Elixir + Erlang/OTP

**Shipped:** unreleased (branch `erlang-elixir-backends`)
**Phase 4。BEAM スタック投入。`requires = ["erlang"]` 初の install-time 依存。**

## なぜ

Phoenix / LiveView / Nerves / 分散システムの Elixir コミュニティ需要。
qusp が「functional/concurrent stack も用意できる」ことの証明。

## 当初計画からの方針転換（重要）

[on-hold 時の計画](../GRAND_PLAN.md) では Erlang を **kerl 相当の source
build**（`./configure && make`、5–15 分）と想定していた。実装着手時に
上流を実測したところ、計画の前提が複数間違っていたため設計を全面的に
組み替えた:

| 当初計画 | 実物 | 採った設計 |
|---|---|---|
| Erlang は source build | `erlef/otp_builds` が **macOS prebuilt tarball** を公開 | prebuilt を pull（qusp の prebuilt-first 哲学に合致、source build の例外カテゴリを増やさずに済んだ） |
| Erlang を `SHA256.txt` で検証 | upstream は `SHA256.txt` を出さず **`.sigstore` バンドルのみ** | DSSE in-toto provenance の `subject[].digest.sha256` を取り出して照合 |
| `Install -minimal` で relocation | prebuilt に **`Install` スクリプトは無い**（フラット展開） | launcher script の `find_rootdir "$0" "<build時パス>"` フォールバックを実インストール先へ書き換え |
| Elixir を `SHA512SUMS` + sha512 で検証 | upstream は per-asset **`.sha256sum` サイドカー**（sha256） | gleam と同形の `.sha256sum` サイドカー検証に統一 |

教訓: **上流のアセットレイアウトは着手時に必ず実測する。** 計画段階の
推測（source build / SHA256.txt / Install / SHA512SUMS）は 4 つすべて外れた。

## 確定した設計

### Erlang/OTP backend (`erlang.rs`)

- Source (macOS, 本命): `erlef/otp_builds` の `OTP-<ver>` リリース、
  `otp-<triple>.tar.gz`。検証は `<asset>.sigstore` の DSSE provenance に
  attest された sha256。Sigstore 署名チェーン（Fulcio/Rekor）のフル検証は
  v1.0 ロードマップ送り。
- Source (Linux glibc): `erlef/otp_builds` は Linux 成果物を出さないので、
  EEF の `builds.hex.pm`（`setup-beam` と同じ供給元）を使う。
  arch ∈ {amd64, arm64} × flavor `ubuntu-{20,22,24}.04`、検証は
  `builds.txt` の4列目 sha256。tarball は source-style（`Install`
  スクリプト同梱）なので relocate_otp の Install 分岐で配置。
  flavor は `/etc/os-release` から推定、`QUSP_OTP_UBUNTU` で上書き可。
  在庫は flavor ごとに違うので候補連鎖でフォールバック
  (`linux_flavor_candidates`、OpenSSL メジャー境界を尊重)。
  **Ubuntu CI で install→run→farm まで実行時検証済み**(`QUSP_E2E_LINUX=1`)。
  mac 開発機では Linux バイナリを実行できないため CI 検証に依存。
  実環境で初めて出た3バグ(builds.txt 先頭空行でのパーサ中断、
  Install へ未存在パスを渡していた件、flavor 在庫差)を修正して green 化。
- musl(Alpine 等)/ Windows / 非対応 arch は明示メッセージで bail。
- relocation: prebuilt は relocatable 設計（`bin/erl` が `find_rootdir`
  で動的解決）。ただし farm symlink（`~/.local/bin/erl`、OTP root の外）
  からは root を辿れず build 時フォールバックに落ちるため、`bin/` と
  `erts-*/bin/` の launcher script のフォールバックを実インストール先へ
  書き換える。`Install` スクリプトを持つ source/legacy tree なら従来どおり
  `./Install -minimal` を実行。
- `build_run_env` で `ERL_ROOTDIR` を明示注入（`qusp run` / hook 経路）。

### Elixir backend (`elixir.rs`)

- Source: `elixir-lang/elixir` の `v<ver>`、`elixir-otp-<major>.zip`
  （BEAM bytecode はアーキ非依存だが OTP メジャーに紐づく）。
- `requires = ["erlang"]`。インストール済みの最新 OTP メジャーを検出して
  対応 zip を取得。該当 zip が無ければ release API から利用可能メジャーを
  列挙して actionable に案内。
- 検証: `<asset>.sha256sum` サイドカー（sha256）。
- `build_run_env` で elixir/bin + 最新 erlang/bin を PATH 前置（mix/iex/
  elixir launcher が実行時に `erl`/`escript` を叩くため）。

### orchestrator: install-time 依存順

Elixir は**インストール時**にインストール済み Erlang の OTP メジャーを
読むため、`requires` を満たすには Erlang が先に入っている必要がある。
`execute_install_plans` を `requires()` ベースのトポロジカル・レイヤー
実行に変更（レイヤー内は並列、レイヤー間は逐次）。`qusp install`
（erlang+elixir 同時 pin）でも Erlang → Elixir の順が保証される。依存が
失敗したレイヤーの依存元はスキップして失敗計上。

（Kotlin/Scala/Groovy/Clojure の `requires=["java"]` は **run-time** 依存
だったので順序不問だった。Elixir が初の install-time 依存。）

## 非ゴール

- Hex（Elixir package manager）の管理。Mix の責務。
- Phoenix / Nerves の framework 管理。
- Erlang の Windows / musl 対応、および Linux ランタイムの mac 上での検証
  （Linux は CI 検証前提の experimental）。

## 実装

1. `crates/qusp-core/src/backends/erlang.rs`（prebuilt + sigstore + relocation）
2. `crates/qusp-core/src/backends/elixir.rs`（`.sha256sum` + requires=erlang）
3. `crates/qusp-core/src/orchestrator.rs`（`layer_install_plans` で依存順）
4. `scripts/e2e/erlang.sh`（install + erl + escript + **farm relocation**）
5. `scripts/e2e/elixir.sh`（cross-backend dep + 依存順 install + mix + run）
