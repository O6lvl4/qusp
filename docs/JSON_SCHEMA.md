# qusp JSON output schema

Phase 5 (Hospitality Parity) audit row R1 で導入。
`qusp <subcommand> --output-format json` で出力される構造化 JSON の
schema を doc 化する。

## Stability contract

JSON schema は qusp の **stability contract** に含まれる。

- **Additive 変更** (新フィールド追加) ─ minor version で許容
- **破壊的変更** (rename / remove / type 変更) ─ major version 必須
- フィールド省略時の挙動: `Option` 型は `null` で出力 (`"version": null`)

CI / scripting / IDE plugin / 別ツール bridge から依存して構わない。

## サブコマンド別 schema

### `qusp backends`

```json
{
  "backends": [
    { "id": "python" },
    { "id": "lua" }
  ]
}
```

### `qusp list <lang>` / `qusp list <lang> --remote`

```json
{
  "lang": "python",
  "scope": "installed",
  "versions": [
    { "version": "3.13.0" },
    { "version": "3.12.7" }
  ]
}
```

`scope` ∈ `"installed"` | `"remote"`。

### `qusp current [<lang>]`

```json
{
  "backends": [
    {
      "backend": "python",
      "version": "3.13.0",
      "source": ".python-version",
      "source_path": "/abs/path/to/.python-version"
    },
    {
      "backend": "ruby",
      "version": null,
      "source": null,
      "source_path": null
    }
  ]
}
```

引数なしで全 backend を返す。`version` / `source` / `source_path` は
pin が無いとき `null`。

### `qusp doctor`

```json
{
  "qusp_version": "0.25.0",
  "paths": {
    "data": "/Users/.../qusp",
    "config": "/Users/.../qusp",
    "cache": "/Users/.../qusp/cache"
  },
  "backends": [
    { "id": "python", "installed_count": 2 },
    { "id": "ruby", "installed_count": 0 }
  ]
}
```

将来追加予定 (Phase 5 後半): `shell_hook_installed`, `warnings`, `errors`。
追加は additive、既存フィールドは保持。

### `qusp dir <kind>`

```json
{
  "kind": "cache",
  "path": "/Users/.../qusp/cache"
}
```

`kind` ∈ `"data"` | `"config"` | `"cache"`。

text モードは bare path を出すので `cd "$(qusp dir data)"` 等の
shell 慣用は維持。

### `qusp outdated`

```json
{
  "entries": [
    {
      "backend": "python",
      "status": "outdated",
      "current": "3.12.7",
      "latest": "3.13.0"
    },
    {
      "backend": "go",
      "status": "up_to_date",
      "current": "1.26.2",
      "latest": "1.26.2"
    },
    {
      "backend": "node",
      "status": "unknown",
      "current": "22.9.0",
      "latest": null
    }
  ]
}
```

`status` ∈ `"up_to_date"` | `"outdated"` | `"unknown"` (上流 API 失敗時)。

## サブコマンドが現状非対応のもの

`--output-format json` を side-effect 系コマンドに渡しても
**silent ignore** (text 出力のまま)。Phase 5 後半で対応予定:

- `qusp install` ─ `{"installed": [...], "skipped": [...], "elapsed_ms": N}` 風
- `qusp sync` ─ 同上
- `qusp tree` ─ project tree の構造化 (Phase 5 後半)

`qusp run` / `qusp x` は exec する性質上、JSON 出力対象にならない。

## 例: jq で使う

```bash
# 全 backend の id 列挙
qusp backends --output-format json | jq -r '.backends[].id'

# python の installed 一覧
qusp list python --output-format json | jq -r '.versions[].version'

# 現在 cwd で pin されてる backend だけ抽出
qusp current --output-format json \
  | jq -r '.backends[] | select(.version != null) | "\(.backend) \(.version)"'

# outdated になってるものだけ
qusp outdated --output-format json \
  | jq -r '.entries[] | select(.status == "outdated") | "\(.backend): \(.current) → \(.latest)"'

# data dir の path
qusp dir data --output-format json | jq -r '.path'
```
