# `qusp upgrade`

**優先度:** Phase 2 (1.x)
**前提:** outdated コマンド既存

## 問題

`qusp outdated` で「↑ rust 1.85.0 → 1.95.0」と出ても、それを apply するには:

1. qusp.toml を手で editor で開く
2. version を書き換える
3. `qusp sync` する

3 ステップ。`qusp upgrade [<lang>...]` 一発でやりたい。

## やること

```
qusp upgrade               # 全 lang を outdated → manifest bump → sync
qusp upgrade rust          # rust だけ
qusp upgrade --dry-run     # 何が起きるか表示のみ
qusp upgrade --major       # major bump も許可 (default は patch only)
```

実装の中身: outdated を流用、manifest の `[lang] version` を書き換え、sync を呼ぶ。

## 設計上の悩み

- `[java] version = "21"` のような major-only pin は 21 が outdated じゃないので何も起きない。
  → いいことにする (LTS major pin の意図を尊重)。
- range spec が入ったら、range 内 latest に bump、というセマンティクスになる。
