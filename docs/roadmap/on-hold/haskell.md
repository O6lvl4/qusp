# Haskell (via ghcup)

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (ghcup wrap は妥当、ただし subprocess 依存)
**前提:** subprocess freeloading 例外の判断 (qusp の哲学を一部緩める)

## なぜ

functional 系 / academic / formal verification 系の中心。
GHC / Cabal / Stack / HLS の組合せが複雑なのでそこを qusp で見えるようにする価値あり。

## 設計

Haskell の世界には **ghcup** という公式 bootstrap installer が既に存在する:
- GHC (compiler), cabal-install, Stack, HLS の全部を1コマンドでインストール・切替
- インストール先 `~/.ghcup/bin/` を PATH に入れて使う
- ghcup 自身は単一の Rust binary (前は Haskell 製、最近 Rust に置換)

### qusp の役割

- **新 backend `haskell`** が ghcup を制御する形:
  - Source: `https://downloads.haskell.org/~ghcup/{version}/x86_64-{os}-ghcup-{version}` (ghcup binary 本体)
  - Verification: `https://downloads.haskell.org/~ghcup/{version}/SHA256SUMS`
  - qusp は ghcup を qusp 管理ディレクトリ (e.g. `versions/haskell/{ghcup-ver}/`) に置く
  - 各 GHC version は qusp が ghcup に dispatch して install:
    `qusp install haskell 9.10.1` → `<store>/ghcup --install-base=<store> install ghc 9.10.1`

`qusp.toml`:
```toml
[haskell]
version = "9.10.1"      # GHC の version
ghcup_version = "0.1.30" # ghcup 自身の version (optional, latest がデフォ)
```

## 設計上の悩み

- **subprocess freeloading 原則違反?** ghcup は qusp が自前で install したものなので「他人のシステム install に依存」じゃない。**「qusp が install した tool に dispatch」は許容範囲**として、CLAUDE.md / ARCHITECTURE.md で明示。
  - 同じ判断を後続 OCaml/opam, Scala/Coursier に流用可。
- **Cabal / Stack / HLS の pin** は別途 `[haskell.tools]` に書きたい。ghcup 経由で install できるが、qusp の registry に curated として置く方が UX 統一。

## 非ゴール

- ghcup の置き換え (qusp が自前で GHC を build する)。GHC は build 時間が長く、bootstrap も難しい。
- Cabal package 管理。Cabal の責務。
- Hackage との直接対話。

## 実装ステップ

1. `crates/qusp-core/src/backends/haskell.rs` (ghcup binary を install + 各 sub-tool を dispatch)
2. ghcup 自体の version pin を保持
3. `[haskell.tools]` で `cabal` / `stack` / `hls` を curated に
4. e2e/haskell.sh (ghcup の中で GHC が build されるので CI で 5-10 分かかる)
