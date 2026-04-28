# Julia

**優先度:** Phase 4 (2.x+)
**難易度:** 低 (prebuilt あり、シンプル)

## なぜ

科学計算・数値最適化・データサイエンスで Python/R に対する第三勢力。
2026 で Julia 1.x が成熟、研究現場で十分使える状態。

## 設計

- **Source:** `https://julialang-s3.julialang.org/bin/{os}/{arch}/{minor}/julia-{version}-{os_arch}.tar.gz`
  - macOS: `https://julialang-s3.julialang.org/bin/mac/aarch64/1.10/julia-1.10.4-macaarch64.tar.gz`
  - Linux: `https://julialang-s3.julialang.org/bin/linux/x64/1.10/julia-1.10.4-linux-x86_64.tar.gz`
- **Verification:** 同ディレクトリに `.sha256` ファイル / `julia-{version}.sha256` (確認要)。
  もしくは `https://julialang-s3.julialang.org/bin/checksums/julia-{version}.sha256` (CDN aggregator 形式)
- **Triple naming:** `mac` + `aarch64`, `linux` + `x86_64`, `linux` + `aarch64`, `winnt` + `x64`
- **Layout:** tarball 展開後 `julia-{version}/bin/julia` + `lib/` + `share/julia/stdlib`
- **Detect:** `.julia-version`

## 設計上の悩み

- **URL の `{minor}` (例: `1.10`) を version からどう導出するか**: `1.10.4` から `1.10` を切り出す。`Version::major_minor()` ヘルパーが要る (現状の Version newtype は opaque、minor extract は後で考える)。今は `version` 文字列から `split('.').take(2).join('.')`。
- **Channel-based pin** (`julia = "1.10"` で latest 1.10.x を解決): Phase 2 の range specs と組み合わせ。
- macOS Universal binary (Intel + arm64 同梱) もある、qusp は arch-specific を選ぶ。

## 非ゴール

- Julia の package manager (Pkg) 管理。Julia 内部 REPL の責務。
- JuliaUp (Microsoft Store / Windows) 経由の install。qusp は CDN 直接。

## 実装ステップ

1. `crates/qusp-core/src/backends/julia.rs`
2. minor 抽出ヘルパ (Version::major_minor() を types.rs に追加)
3. e2e/julia.sh
