# Python Tools via uv Routing

**優先度:** Phase 3 (2.x)
**前提:** v0.5.0 release notes で予告済 (未実装)

## 問題

Python は curated tool registry が空。uv 自体は素晴らしい tool installer:

```
uv tool install ruff
uv tool install black
uvx pre-commit
```

qusp 側で別途 registry を作るより、uv に dispatch する方が筋がいい。

## やること

1. `python::knows_tool(name)` を `uv tool list-installable` 風に常に true にする (or 主要なものだけ)
2. `python::resolve_tool(http, name, spec)` で uv の resolver を呼ぶ (subprocess は許容?)
3. install_tool は `uv tool install <name>@<version>` を呼ぶ
4. `qusp run <bin>` で uv が install した bin を見つけられるよう、build_run_env が uv の bin path を含む

## 設計上の悩み

- 「subprocess freeloading 禁止」原則と衝突。uv も「publisher が出してる tool」と捉えられるか?
- uv の更新は qusp の責務外。`qusp install python` は uv も入れる?
