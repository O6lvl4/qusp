# Inline script metadata (PEP 723 風)

**Phase 5 (Hospitality Parity)。**
**前提:** v0.24.0 の `qusp x` extension-routing。

## なぜ

uv は PEP 723 (Python script metadata) 対応で:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx"]
# ///
import httpx
```

を `uv run script.py` で読んで Python 3.12 と httpx を 1 動詞で揃える。
これに相当する cross-language inline metadata を qusp も用意したい。

## 設計案

### 文法

各言語の comment syntax に合わせて統一プレフィクス `# qusp:` (or
`-- qusp:` / `// qusp:` / `;; qusp:`) を引数解釈する。最初の N 行
(N = 30 程度) を scan、初出 hit を採用。

例: lua
```lua
-- qusp: lua = 5.4.7
print("hi")
```

例: ts
```ts
// qusp: deno = 2.0.0
console.log("hi")
```

例: clojure
```clojure
;; qusp: clojure = 1.12.4.1618
(println "hi")
```

例: haskell
```haskell
-- qusp: haskell = 9.10.1
main = putStrLn "hi"
```

### 拡張形 (将来)

```python
# qusp:
#   python = "3.12.0"
#   tool.ruff = "0.6.9"
```

YAML 風のマルチライン形式。tool dependency の記述まで踏み込む段階で
PEP 723 と同じ shape に近づける。

### Resolution priority への組込み

現状 (v0.24.0 script.rs):
1. qusp.toml の `[<lang>] version`
2. `.<lang>-version`
3. `list_installed` の最新
4. curated default

新:
0. **inline metadata** (script 自身の `# qusp: <lang> = <v>`)
1. qusp.toml
2. `.<lang>-version`
3. list_installed の最新
4. curated default

inline metadata が最優先になる。理由: script 自身が再現性を声明してる
ので、外部 manifest よりそれを尊重するのが PEP 723 spirit。

## 設計上の悩み

- **Comment prefix の検出**: 言語ごとに `#`, `--`, `//`, `;;`, `<!--`
  などバラバラ。`extension_to_lang` で言語決定 → 言語ごとの comment
  syntax で metadata 行を scan、というシーケンスが筋良い。
- **Metadata の書き方統一**: 今は `# qusp: lua = 5.4.7` 1 行で十分だが、
  将来 tool / dependency まで拡張するときに syntax を後悔しない形で
  始めたい。最初は 1 行 key=value、将来的に YAML block (`# /// qusp` ~ `# ///`
  風) を許容する伸び代を残す。
- **Security**: script を読むだけなので exec 危険性は無いが、明確に
  「qusp は script 内 metadata を読んで version pin に使うが、それ以外
  の解釈はしない」を docs で明示。

## 非ゴール

- Tool / dependency / lock 統合 (まずは version pin だけ)
- script 内 PEP 723 完全互換 (Python の `# /// script` ~ `# ///`
  block も認識する形は将来検討、初版は qusp 専用 prefix)

## 実装ステップ

1. `crates/qusp-cli/src/script.rs` に `read_inline_metadata(script_path,
   lang) -> Option<String>` 追加
2. `resolve_script_version` の優先順位 0 に挿入
3. 各 backend の comment syntax を表化:
   - `#` ─ python, ruby, lua (`--` は `#!` の後に)
   - `--` ─ lua, haskell, sql
   - `//` ─ go, rust, c, ts, js, scala, clojure (no, clojure は `;;`)
   - `;;` ─ clojure
   - `'` ─ vbs (qusp 範囲外)
   一旦 lua / python / ruby / haskell / clojure / scala / ts / js /
   go / zig / dart / crystal / java / kotlin / groovy / julia の 16
   をハードコード
4. unit tests: 各 comment syntax で正しく抽出
5. e2e: inline metadata 持ち script を `qusp x` して期待 version で
   run することを assert

## CLI 例

```
$ cat hello.lua
-- qusp: lua = 5.4.5
print("oldskool")

$ qusp x ./hello.lua            # Lua 5.4.5 を install して exec
✓ lua 5.4.5 installed for ephemeral run
oldskool
```
