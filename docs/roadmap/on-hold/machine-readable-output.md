# Machine-readable JSON output

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** R1

## なぜ

実測 (audit 2026-04-28):

```
$ uv python list --output-format json
[{"key":"cpython-3.14.4-macos-x86_64-none","version":"3.14.4",
  "version_parts":{"major":3,"minor":14,"patch":4},
  "path":"/usr/local/bin/python3.14",
  "url":null,"os":"macos","variant":"default",
  "implementation":"cpython","arch":"x86_64","libc":"none"}, ...]
```

vs

```
$ qusp list python --json
error: unexpected argument '--json' found
```

uv は `list` / `python find` / `tool list` / `cache info` 等の introspection 系を JSON で吐ける。これにより:

- CI script で `jq '.[] | select(.installed)'` のようにフィルタ可能
- 別ツール (mise → qusp 移行時等) との bridge
- editor / IDE plugin が qusp 状態を読める
- regression test がぺら一行 grep じゃなく structured assertion で書ける

qusp は machine-readable channel が **完全に欠落**。`--json` flag 追加は Phase 5 hospitality の重要な要素。

## 設計案

### CLI

`--output-format <format>` で uv 同形:

```
qusp list python --output-format json
qusp current --output-format json
qusp outdated --output-format json
qusp tree --output-format json
qusp doctor --output-format json
qusp backends --output-format json
qusp dir cache --output-format json
```

format = `text` (default、人間向け) | `json` | `json-lines` (1 record per line)。

### Schema

各 subcommand で安定 schema を持ち、stability contract に入れる。

#### `qusp list <lang>`

```json
[
  {
    "backend": "python",
    "version": "3.13.0",
    "install_dir": "/Users/.../qusp/python/3.13.0",
    "store_sha_prefix": "abc123def456",
    "current_for_cwd": false,
    "current_source": null,
    "implementation": "cpython",
    "variant": null,
    "distribution": null
  },
  ...
]
```

#### `qusp current [lang]`

```json
{
  "backends": [
    {
      "backend": "python",
      "version": "3.13.0",
      "source": ".python-version",
      "source_path": "/path/to/.python-version",
      "resolved_path": "/Users/.../qusp/python/3.13.0/bin/python"
    }
  ]
}
```

#### `qusp doctor`

```json
{
  "qusp_version": "0.24.0",
  "build_rev": "a1b2c3d4e5",
  "build_date": "2026-04-28",
  "data_dir": "/Users/.../qusp",
  "cache_dir": "/Users/.../qusp/cache",
  "config_dir": "/Users/.../qusp",
  "shell_hook_installed": false,
  "backends": [
    {"id": "python", "installed": 2, "default_version": "3.13.0"},
    ...
  ],
  "warnings": [],
  "errors": []
}
```

### 実装

`crates/qusp-cli/src/output.rs` (新規) で format dispatch:

```rust
pub enum OutputFormat { Text, Json, JsonLines }

pub fn print<T: serde::Serialize + std::fmt::Display>(format: OutputFormat, item: &T) {
    match format {
        OutputFormat::Text => print!("{}", item),
        OutputFormat::Json => {
            let s = serde_json::to_string_pretty(item).unwrap();
            println!("{}", s);
        }
        OutputFormat::JsonLines => {
            let s = serde_json::to_string(item).unwrap();
            println!("{}", s);
        }
    }
}
```

各 subcmd の output struct を `Serialize + Display` で 2 重実装。

### Stability contract

format=json の schema は `docs/JSON_SCHEMA.md` で固定。
- additive 変更 (新 field 追加) は minor version で OK
- 破壊的変更 (rename / remove) は major version 必須
- これは qusp 1.0 stability contract の範囲

## 設計上の悩み

- **`Display` と `Serialize` の二重維持**: 大半は `Display` の方が
  自動派生できないので手書き。代替: `Tabled` crate で TUI 用 row も
  derive。
- **エラーも JSON で吐くか**: `--output-format json` 時は stderr も
  `{"error": "...", "code": "..."}` で出すべき。CI 用途で重要。
- **後方互換**: 既存 `text` モードを 100% 維持、JSON は purely
  additive。

## 非ゴール

- YAML output (JSON で十分、YAML は後)
- Protobuf / msgpack (overkill)
- streaming JSON (現状の出力量で不要)

## 実装ステップ

1. `crates/qusp-cli/src/output.rs` 新規 + `OutputFormat` enum
2. global `--output-format` flag (alias `--format`)
3. subcmd ごとに output struct + `Serialize` derive + `Display` 手書き
4. error path も JSON 化 (`--output-format json` 時の stderr 形式)
5. `docs/JSON_SCHEMA.md` で schema 文書化
6. e2e で `qusp list python --output-format json | jq` 系の test
