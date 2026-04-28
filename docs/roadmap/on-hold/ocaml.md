# OCaml (via opam)

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (Haskell/ghcup と同形)
**前提:** ghcup / coursier と同じ「qusp が install した bootstrap installer に dispatch」判断を採用

## なぜ

functional + 型推論パワー + コンパイラ研究の重鎮。
ReScript / Reason / MirageOS / Coq などの周辺エコシステム。

## 設計

OCaml の bootstrap は **opam**:
- opam binary は単一実行ファイル、`https://github.com/ocaml/opam/releases` でリリース
- `opam init` でユーザー directory に prefix を作って switch (= 隔離されたコンパイラインストール) を生成
- `opam switch create 5.2.0` で OCaml 5.2.0 をビルド+install (subprocess で `ocaml-base-compiler` を build、~10 分)

### qusp の役割

- **新 backend `ocaml`** が opam を制御:
  - Source (opam): `https://github.com/ocaml/opam/releases/download/{version}/opam-{version}-{triple}` (single binary)
  - Verification: GitHub release sha256
  - qusp は opam を qusp 管理ディレクトリに置く
  - 各 OCaml version は opam に dispatch:
    `qusp install ocaml 5.2.0` → `<store>/opam switch create --root=<store> 5.2.0`

```toml
[ocaml]
version = "5.2.0"
opam_version = "2.3.0"  # optional
```

## 設計上の悩み

- **OCaml の switch == per-project compiler install**: opam の native concept。qusp の `[ocaml] version = X` ↔ opam switch がうまく対応するか
- **OCaml は base compiler の build が必須** (Haskell より状況が悪い、prebuilt がほぼ無い)。Build 時間 5-15 分
- **dune (build tool) や OPAM packages** の pin 需要が高いが、それは package layer。qusp は compiler のみ。

## 非ゴール

- opam packages の pin (Phase 5 で考える)。
- dune の version 管理 (opam 経由でいい)。

## 実装ステップ

1. `crates/qusp-core/src/backends/ocaml.rs` (opam wrap)
2. opam の root を qusp の content-addressed store 内に
3. e2e/ocaml.sh — CI で base compiler の build を待つ
