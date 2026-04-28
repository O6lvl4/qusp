# `qusp x <script>` extension-routing

**Shipped:** v0.24.0
**Tag:** v0.24.0
**Phase 5 第一弾。新コンセプト「Hospitality Parity」の最初の証拠。**

## なぜ Phase 5 の先頭か

uv の核心体験 `uv run hello.py` を qusp の 18 言語横断 backend で
再現する最小実装。ここを立てると、qusp の position が
「mise/asdf より厳しい / uv より広い」というユニーク座標として
言えるようになる。

## 設計

`qusp x` の引数 argv[0] が **既存ファイル + 既知拡張子** に該当した場合、
従来の tool-name dispatch ではなく **language-runner dispatch** に分岐する。

```
qusp x ./hello.lua             → lua hello.lua
qusp x ./Hello.scala           → scala Hello.scala
qusp x ./hello.hs              → runghc hello.hs
qusp x ./script.ts             → deno run script.ts
qusp x ./args.lua a b c        → lua args.lua a b c       (passthrough)
qusp x not-a-real-tool         → existing tool dispatch (fallback)
```

非破壊: qusp.toml / qusp.lock を**書き込まない** (ephemeral 名通り)。

## 拡張子マッピング (16 言語)

スクリプト言語:
- `.py / .pyi` → python ─ `python <f>`
- `.lua` → lua ─ `lua <f>`
- `.rb` → ruby ─ `ruby <f>`
- `.jl` → julia ─ `julia <f>`
- `.groovy` → groovy ─ `groovy <f>`

Single-file launcher のあるソース言語:
- `.java` → java ─ `java <f>` (JEP 330, JDK 21+)
- `.kts` → kotlin ─ `kotlin -script <f>`
- `.scala / .sc` → scala ─ `scala <f>` (3.5+ scala-cli)
- `.clj / .cljc` → clojure ─ `clojure <f>`
- `.hs` → haskell ─ `runghc <f>`

JS/TS:
- `.js / .mjs / .cjs` → node ─ `node <f>`
- `.ts / .mts / .cts` → deno ─ `deno run <f>` (組込み TypeScript)

`<lang> run` 系:
- `.go` → go ─ `go run <f>`
- `.zig` → zig ─ `zig run <f>`
- `.dart` → dart ─ `dart run <f>`
- `.cr` → crystal ─ `crystal run <f>`

意図的に未対応:
- `.rs` (rust scripts は cargo 前提、単一 file 概念が薄い)
- `.kt` (完全 compile が必要、`-script` で動かない)
- bun は明示 pin (.js/.ts は node/deno 既定)

## バージョン解決

優先順位:
1. `qusp.toml` の `[<lang>] version`
2. `.<lang>-version` (backend.detect_version)
3. `list_installed` の最新
4. crates/qusp-cli/src/script.rs の curated `default_version`

(4) は cmd_init の version map と完全に同期 (release prep で確認)。

## Cross-backend 依存の扱い

`Backend::requires` (Kotlin/Scala/Clojure/Haskell が Java 必須など) は
**自動で pull しない**。`qusp x ./Hello.scala` で Java 未 install なら
Scala の launcher が `java: command not found` で失敗する。これは
ephemeral の特性 (qusp.toml を書かない、transitive 解決もしない) として
受け入れる: ユーザは `qusp install java 21` を別途打つか、qusp.toml に
両方 pin して `qusp run` を使う。

## 実装

`crates/qusp-cli/src/script.rs` (新規):
- `extension_to_lang(path)` ─ ファイルパス → 言語 id
- `script_run_argv(lang, script)` ─ 言語ごとの canonical argv
- `default_version(lang)` ─ qusp release 時点の "latest reasonable"
- `detect_script_invocation(argv0)` ─ 「実 file + 既知拡張子」の AND 検出
- `resolve_script_version(...)` ─ 4 段優先順位のバージョン決定
- `run_script(...)` ─ install + env 構築 + exec

`crates/qusp-cli/src/main.rs` の `cmd_x` 冒頭で
`detect_script_invocation` が Some を返したら `run_script` に分岐、
そうでなければ既存の tool-routing に fall through。

## 4 unit tests

- `extension_to_lang` が 18 言語の主要拡張子 + 大文字版 + 未対応拡張子
  (`.rs`, `.kt`, `noext`) を網羅、未対応で `None` を返す
- `script_run_argv` が 4 代表 (lua/deno/haskell/kotlin) の canonical argv
- `default_version` が `extension_to_lang` が返しうる全 lang を被覆
  (新言語追加時の検出網)
- `detect_script_invocation` が「実 file + 既知拡張子」の AND 条件
  (file 不在は None でフォールスルー、bare command 名も None)

## Smoke + e2e

- `qusp x ./hello.lua` (fresh HOME = lua 未 install) → install + exec
- 2 度目は既 install を reuse、瞬時
- `qusp x ./args.lua alpha beta` → arg passthrough
- `qusp x not-a-real-tool-or-script` → tool dispatch 経路に fall-through

cargo test --release: qusp-core 57/57 + qusp-cli 4/4 = 61 tests passing。
e2e/x_script.sh は make/cc が無い host で skip 77。
e2e.sh DEFAULT/FAST 両方に追加。

## 何が立ったか

これで「fresh machine で `qusp x ./<anything>.{lua, py, scala, ...}` が
auto-install + exec で動く」体験が 16 言語に渡って成立した。
**uv-class hospitality, broad** の最初の証拠。

残りの hospitality 拡張は `on-hold/hospitality-parity.md` 参照。
