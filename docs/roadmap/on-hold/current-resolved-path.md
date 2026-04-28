# `qusp current --resolved` で絶対 path 表示

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** B3 ─ Resolve current

## なぜ

実測 (audit 2026-04-28):

```
$ uv python find
/Users/o6lvl4/.local/share/mise/installs/python/3.11.13/bin/python
```

vs

```
$ qusp current python
python (none)
```

(or with `.python-version`:)
```
python 3.13.0 (from .python-version)
```

qusp の `current` は **version 文字列 + source** を返すが、
**絶対 path** を返さない。ユーザが「`which python`」相当の確認
(対象の interpreter binary を直接叩きたい / IDE 設定にコピペしたい /
script の shebang に書きたい) ができない。

uv の `python find` はまさに **絶対 path** を出すコマンド。これに相当
する `qusp current --resolved` を追加する。

## 設計案

### CLI

```
$ qusp current python
python 3.13.0 (from .python-version)

$ qusp current python --resolved
/Users/o6lvl4/Library/Application Support/dev.O6lvl4.qusp/python/3.13.0/bin/python

$ qusp current --resolved
java     /.../java/temurin-21/bin/java
node     /.../node/22.9.0/bin/node
python   /.../python/3.13.0/bin/python
```

### implementation

各 backend が `Backend::main_binary_name() -> &'static str` (既存
`build_run_env` で path を組み立てるロジックの一部) を露出。
CLI 層で `<install_dir>/bin/<binary_name>` を組み立てて print。

```rust
pub trait Backend {
    /// The canonical "main binary" exposed by qusp run.
    /// e.g. "python" for python, "go" for go, "scala" for scala.
    fn main_binary_name(&self) -> &'static str { self.id() }
    ...
}
```

multi-binary backend (haskell の `ghc` / `runghc` / `ghci`) は
`main_binary_name = "ghc"` がデフォルト、`--bin <name>` で override 可。

## 設計上の悩み

- **既存 `current` の出力フォーマット維持**: human-readable な
  "version (from source)" は壊さない、`--resolved` は machine-readable
  に切る。`--resolved --plain` で path のみ単独出力。
- **絶対 path vs symlink 解決**: qusp の data dir は symlink chain を
  持つ (data → store)。`canonicalize` で chain を解決して出すか、
  user-facing は data dir 側 (symlink) のまま見せるか。後者の方が
  user expectation に近い。

## 非ゴール

- `which` の置き換え (これは shell 機能)
- 全 sub-binary の列挙 (`ghc-9.10.1`, `ghci`, `runghc` など) ─ 後で
  必要なら別 subcommand `qusp which <bin>`

## 実装ステップ

1. `Backend::main_binary_name` trait method 追加 (default = self.id())
2. multi-binary backend で override (haskell, kotlin など)
3. `cmd_current` に `--resolved` flag、絶対 path 組み立て
4. unit test
5. e2e で `qusp current python --resolved` の出力 assert
