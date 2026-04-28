# Cross-language tool install registry

**Phase 5 (Hospitality Parity)。**
**前提:** Phase 3 の Python tools-via-uv を内包しつつ拡張する。

## なぜ

uv の `uv tool install ruff` は Python ツールを isolated venv に置いて
shim 不要で `ruff` が叩ける。これに相当する cross-language 体験を
qusp が用意できると、language toolchain manager から **toolchain +
tool ecosystem manager** へ position が広がる。

## 設計案

```
qusp tool install ruff           → Python の uv に dispatch (ruff は ruff package)
qusp tool install gopls          → Go 経由 (gv-core registry に既登録)
qusp tool install scalafmt       → Scala 経由 (Coursier or coursier-bootstrapped JAR)
qusp tool install prettier       → Node 経由 (npm i -g)
qusp tool install hls            → Haskell 経由 (ghcup install hls)
qusp tool install cabal          → Haskell 経由 (ghcup install cabal)
qusp tool install ocamlformat    → OCaml 経由 (opam install)
```

### Routing の仕組み

各 backend が **curated tool registry** を持ち、`Backend::knows_tool(name)`
が true を返す backend に dispatch。複数 backend が同名 tool を知ってる
場合 (`prettier` を node も bun も持ってる)、優先順位は backend ごとの
設定 + ユーザの `[tools] preferred = "node"` で resolution。

### Lockfile 統合

`qusp.lock` に tool entry を持つ仕組みは既存 (Go の gv-core 経由の tool
は既に lock される)。これを cross-language で統合する形。

```toml
[tool.ruff]
backend = "python"
version = "0.6.9"
upstream_hash = "..."

[tool.gopls]
backend = "go"
version = "0.16.2"
upstream_hash = "h1:..."
```

### CLI 体験

```
$ qusp tool install ruff
✓ tool ruff 0.6.9 (via python uv) installed
$ qusp tool list
  ruff       0.6.9         python (uv)
  gopls      0.16.2        go
  scalafmt   3.8.3         scala (coursier)
$ qusp tool run ruff check src/    ←  shim なし、直接 exec
```

## 設計上の悩み

- **Python の uv を install するのか subprocess で借りるのか**: 既に
  Phase 3 の python-tools-via-uv で議論中。qusp が uv binary を sha256
  検証して install + dispatch するのが筋 (ghcup wrap と同形)。
- **Tool registry の拡張頻度**: 各 backend で curated set を maintain
  するコストが高い。コミュニティに `[tool.<name>]` で
  `backend = "<id>"` + `package = "<package>"` を qusp.toml で書ける
  ようにすれば extension 不要に。
- **Conflict resolution**: 同名 tool を複数 backend が持つ問題。
  ユーザ pin が無い場合の決定的な priority が要る。
- **`qusp x` との関係**: `qusp x ruff check src/` (現状の tool dispatch)
  と `qusp tool run ruff check src/` がほぼ同じになる。動詞統合の
  検討も別途。

## 非ゴール

- 任意 Maven / npm / PyPI / Hackage / OPAM package の resolve
  (qusp は curated tool に限る、package manager 競合をしない)

## 実装ステップ

1. Phase 3 の python-tools-via-uv を **先に完了**させる (uv binary を
   qusp が wrap する pattern を確立)
2. `qusp tool install/list/run` のサブコマンド追加
3. 各 backend の `knows_tool()` を埋める (Go は既存、他は新規 curated)
4. cross-backend conflict resolution の仕様化
5. `[tool.*]` manifest セクション + lock entry の format 確定
6. e2e: ruff / gopls / scalafmt / prettier の install + run 確認
