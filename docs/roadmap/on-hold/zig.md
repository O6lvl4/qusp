# Zig

**優先度:** Phase 4 (2.x+) — **難易度最低、優先で取りに行く価値あり**
**前提:** なし

## なぜ

2026 の systems-language の現代的選択肢。Bun と同じく「シンプル distribution + 単一 binary」で qusp の既存パターンに 100% 乗る。

## 設計

- **Source:** `https://ziglang.org/download/index.json` が公式 release index (JSON)、各 release に asset URL + sha256 + tarball サイズ
- **Asset URL pattern:** `https://ziglang.org/download/{version}/zig-{os}-{arch}-{version}.tar.xz` (Linux/macOS) or `.zip` (Windows)
- **Triple naming:** `linux-x86_64`, `linux-aarch64`, `macos-x86_64`, `macos-aarch64`, `windows-x86_64`
- **Verification:** sha256 が index.json にインライン記載
- **Layout:** tarball 展開すると `zig-{os}-{arch}-{version}/zig` (single binary) + `lib/std/`、`bin/` を作って symlink するか zip 内側を直接 PATH に
- **Detect version:** `.zig-version` (asdf 互換)

## 設計上の悩み

- **`.tar.xz` 解凍が必要**。anyv-core は `.tar.gz` と `.zip` だけ対応。xz サポート追加 OR `.zip` を使う (全 platform に zip も併載されてるか要確認)
  - 実際は全 platform `.tar.xz` only、Windows のみ `.zip`。**xz サポート追加が現実的**。
  - `xz2` クレート (1.0、bindings to liblzma) を追加するか、`async-compression` の `xz` feature を追加

## 非ゴール

- master branch (nightly) は Phase 1.5 では入れない、stable のみ。

## 実装ステップ

1. `xz2` を qusp-core dep に追加 (or anyv-core に xz extract サポート PR)
2. `crates/qusp-core/src/backends/zig.rs` 新規作成
3. e2e/zig.sh
4. README + qusp init template に zig 追加
