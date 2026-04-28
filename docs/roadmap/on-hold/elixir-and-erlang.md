# Elixir + Erlang/OTP

**優先度:** Phase 4 (2.x+)
**前提:** `Backend::requires` 機構 (Phase 1 完了済)
**難易度:** 高 (Erlang OTP の source build がある)

## なぜ

Phoenix / LiveView / Nerves / 分散システムの elixir コミュニティ需要。
qusp が「functional/concurrent stack も用意できる」ことの証明。

## 設計

### Erlang/OTP backend

prebuilt が薄い。Erlang Solutions の `esl-erlang` apt/yum パッケージはあるが、tarball は無い。
現実は **kerl 相当の source build**:

- Source: `https://github.com/erlang/otp/releases/download/OTP-{ver}/otp_src_{ver}.tar.gz`
- Verification: GitHub release body に sha256 (人間可読、scrape 必要) or `.sha256` sidecar (要確認)
- Build: `./configure && make && make install` を spawn_blocking
- **これは ruby-build / luaの「唯一の例外」カテゴリ**。CLAUDE.md / README で明示。
- Build 時間: 5-15 分

### Elixir backend

- Source: `https://github.com/elixir-lang/elixir/releases/download/v{ver}/Precompiled.zip`
- Verification: release notes にチェックサム or .sha256
- `requires = ["erlang"]`。version pin は `1.18.0-otp-27` のような複合形式 ("Elixir X compiled against OTP Y")
- Layout: zip → bin/elixir, bin/elixirc, bin/mix, bin/iex
- ELIXIR_HOME / build_run_env で PATH に bin/、erlang の env も merge

## 設計上の悩み

- **`1.18.0-otp-27` のような version pin** をどうパースするか。`Version` newtype は trim/non-empty しか保証してないので、Elixir backend が内部で `(elixir_ver, otp_major)` に split する。
- **OTP source build** は Phase 5 (reproducibility) と相性が悪い。binary cache を qusp 側で持つ?
- ユーザーが手元に Erlang をすでに持ってる場合 (homebrew など) の **既存検出** をするか、毎回 build するか。デフォは「qusp 管理外」尊重で既存があればスキップしたいが、誤検出のリスク。

## 非ゴール

- Hex (Elixir package manager) の管理。それは Mix の責務。
- Phoenix / Nerves の framework 管理。

## 実装ステップ

1. `crates/qusp-core/src/backends/erlang.rs` (source build, spawn_blocking)
2. `crates/qusp-core/src/backends/elixir.rs` (`requires = ["erlang"]`, Precompiled.zip)
3. e2e/erlang.sh + e2e/elixir.sh — CI では elixir/erlang prereq install が必要
4. version pin format `1.18.0-otp-27` のパーサ (Elixir backend 内部)
