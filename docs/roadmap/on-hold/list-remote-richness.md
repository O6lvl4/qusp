# List remote richness 強化

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** B1 ─ list remote の質、I1 ─ mise の source column

## なぜ

実測 (audit 2026-04-28) で uv の `python list` は:

```
cpython-3.14.4-macos-x86_64-none      /usr/local/bin/python3.14 -> ../Cellar/python@3.14/3.14.4/bin/python3.14
cpython-3.14.0rc2-macos-x86_64-none   <download available>
cpython-3.13.7-macos-x86_64-none      /usr/local/bin/python3.13 -> ...
cpython-3.13.7+freethreaded-macos-x86_64-none  <download available>
```

各行に:
1. **Implementation tag** (`cpython-`, `pypy-` など)
2. **Variant tag** (`+freethreaded`, `+rc2`)
3. **Install status** (`<download available>` / 既 install path)

qusp の `list python --remote` は:

```
3.13.13
3.13.12
3.12.13
3.11.15
3.10.20
```

bare semver list のみ。これは情報量として 3 段階下:
- どの implementation か (qusp は CPython 固定だが他言語で variant あり)
- 既に手元にあるか
- 何の variant か

mise は別軸で `mise list` (installed のみ) に **source column** を出す:

```
python  3.11.13  ~/.config/mise/config.toml  21
```

「どの設定ファイルが指定したか」が即見えるのが mise の強み。qusp の
list には source 表示が無い。

## 設計案

### `qusp list <lang>` (installed)

新フォーマット:

```
$ qusp list python
python  3.13.0     /Users/.../qusp/python/3.13.0       (current via .python-version)
python  3.13.13    /Users/.../qusp/python/3.13.13
python  3.11.13    /Users/.../qusp/python/3.11.13
```

- col 1: backend id
- col 2: version
- col 3: install path (絶対 path)
- col 4: source / status: `(current via X)` / `(default)` / なし

### `qusp list <lang> --remote` (available)

新フォーマット:

```
$ qusp list python --remote
python  3.13.13   <installed>
python  3.13.12   <download available>
python  3.12.13   <download available>
python  3.12.7    <installed>
python  3.11.15   <download available>
python  3.10.20   <download available>
python  3.10.18   <installed>
```

各 row に install 状態を ad-hoc で付ける。実装上は list_remote の結果を
list_installed で intersect して `<installed>` 注釈。

### Implementation/variant tag (将来)

CPython のみのうちは bare version で十分。Python に PyPy / GraalPython
が混じる、Java で multi-vendor が見える、などになったら uv の
`<impl>-<v>-<host>` 形式に近づける。当面は backend 側で
`list_remote_rich() -> Vec<RemoteVersion>` (struct で impl + variant +
status) を opt-in 提供する形を Trait に追加。

```rust
pub struct RemoteVersion {
    pub version: String,
    pub installed: bool,
    pub variant: Option<String>,    // e.g. "+freethreaded"
    pub implementation: Option<String>, // e.g. "cpython"
    pub source: Option<String>,     // e.g. ".python-version" or qusp.toml path
}
```

backend は default として bare version-string を Vec<String> で返す
従来の API も維持、rich 版は overrideable。

## 設計上の悩み

- **`list_remote` 速度**: uv は cache を使い回してる。qusp の
  `list_remote_rich` も同様に in-process cache (TTL 5min くらい) を持つ。
- **Java multi-vendor**: 既に foojay からの distribution を取れるので、
  `python list java --remote` で `temurin-21 / corretto-21 / zulu-21 /
  graalvm_community-21` 全部 enumerable にできる。これは Java backend
  だけで impl/variant tag の本格活用が出る。
- **Source col の正確性**: `.python-version` を尊重したのか
  `qusp.toml` を尊重したのか、orchestrator の resolution 順を tracking
  する必要がある。`Backend::detect_version` の戻り値 `DetectedVersion`
  に既に `source: String` field があるのでそれを reuse。

## 非ゴール

- パッケージレベル list (`uv pip list` 相当)。Python 専用、qusp 範囲外。
- フィルタ (`--latest-stable` 等) ─ 後回し、basic richness を先に。

## 実装ステップ

1. `Backend::list_remote_rich -> Vec<RemoteVersion>` trait method 追加 (default 実装は既存 `list_remote` を wrap)
2. CLI `cmd_list` を rich 版に書き換え、tabular formatter
3. installed-intersection logic を orchestrator に
4. `--remote` 時は rich format、`--remote --plain` で legacy
5. Java backend を最初に rich 化 (multi-vendor で価値が大きい)
6. e2e で出力フォーマット assert
