# Crystal

**Shipped:** v0.17.0
**Tag:** v0.17.0
**Phase 4 第三弾。GitHub `asset.digest` 経由 sha256 verify を初導入。**

## なぜ

Ruby-like syntax + LLVM compiled + native binary。Lucky framework + Avram ORM のニッチ but 熱い ecosystem。
mise が対応してて qusp に無いと「やる気がない」シグナルになる。

## 設計

- **Source:** `https://github.com/crystal-lang/crystal/releases/download/{version}/crystal-{version}-{n}-{os}-{arch}.tar.gz`
  - 例: `crystal-1.13.1-1-darwin-universal.tar.gz`, `crystal-1.13.1-1-linux-x86_64.tar.gz`
- **Verification:** GitHub release に各 asset の sha256 が body に published、または `.sha256` sidecar (要確認)
- **Triple naming:** `darwin-universal` (macOS は universal binary)、`linux-x86_64`, `linux-aarch64`
- **Layout:** tarball 展開後 `crystal-{version}/{bin/{crystal, shards}, src/, share/}`
- **Detect:** `.crystal-version`

## 設計上の悩み

- **macOS Universal binary**: macOS x86_64 / arm64 共通。qusp の triple マップでは macOS の場合 arch を見ない special-case が要る (or `darwin-universal` を arch 不問で選ぶ)。
- **Crystal は LLVM に依存**: 動作には libllvm が必要 (ただし配布バイナリは静的リンクされてるはず、要確認)。

## 非ゴール

- Shards (Crystal package manager) 管理。Shards の責務。
- Crystal の build (compiler 自身を qusp が build する)。Crystal は self-hosted で bootstrap 困難なので prebuilt 一択。

## 実装ステップ

1. `crates/qusp-core/src/backends/crystal.rs`
2. macOS の universal triple の特殊化
3. e2e/crystal.sh
