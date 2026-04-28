# Did-you-mean fuzzy across all backends

**Phase 5 (Hospitality Parity)。**

## なぜ

Python backend には既に typo / 不存在 version に対する fuzzy 提案が
あり (`done/python-fuzzy-and-partial.md`、v0.8.1)、ユーザ体験として
明確に良い:

```
$ qusp install python 3.12.71
error: python 3.12.71 not found
       did you mean 3.12.7? (closest by edit distance)
                3.13.0?     (latest)
```

これが他 17 backend に無い。Phase 5 では同じ体験を全 backend に展開する。

## 設計案

### 抽象化

各 backend の `list_remote()` は既に Vec<String> を返す。fuzzy 提案は
「ユーザ入力 vs list_remote() の中の最近接 N 件」を表示するだけなので
backend-side の追加実装は不要、`qusp install` の error path で共通化できる。

```rust
// crates/qusp-core/src/fuzzy.rs (新規 candidate)
pub fn suggest_versions(target: &str, available: &[String], n: usize) -> Vec<String> {
    // edit distance + version 距離 (semver-aware) のハイブリッドスコア
    // で上位 n 件を返す。"3.12.71" → "3.12.7" は edit distance 1、
    // semver でも 3.12.7 が近い、というシナリオが両方優位を取る。
}
```

### Error path 改修

`cmd_install` の version not found error を catch して
`suggest_versions` を回し、現在の error message に append:

```
error: scala 3.5.99 not found
       did you mean 3.5.2 or 3.8.3?
       (qusp installs Scala 3.7.0+ — older versions lack .sha256 sidecars)
```

最後の括弧は backend-specific の context (Scala の floor 3.7.0 のような
制約)、各 backend が `version_floor_hint()` 等で optional に提供する形。

## 設計上の悩み

- **list_remote が遅い backend**: foojay (Java) は per-vendor で
  list が大きく、毎エラーで叩くと遅い。In-process LRU cache で
  install session 内 1 回に。
- **Edit distance vs version distance**: "3.12.71" は edit distance
  では "3.12.7" 一択、semver では "3.12.7" と "3.13.0" が両方近い。
  両方上位に出すのが UX 上正解 (latest と closest の両方)。
- **Distribution-aware (Java)**: foojay は `distribution + version` の
  2 軸。version が distribution 由来でしか valid じゃないケースが
  ある。fuzzy は distribution まで踏み込む必要があるかも。

## 非ゴール

- `qusp install` 以外の path での fuzzy (まずは install path 専用)
- Tool 名 fuzzy (Phase 3 / cross-language tool registry の領域)

## 実装ステップ

1. `crates/qusp-core/src/fuzzy.rs` ─ pure function、unit tests heavy
2. `cmd_install` の error path で suggest_versions を組み込む
3. backend ごとの version_floor_hint optional method を追加 (Trait
   default = None、必要な backend だけ override)
4. e2e: 不存在 version で install を呼んで、error 出力に "did you mean"
   が含まれることを assert
