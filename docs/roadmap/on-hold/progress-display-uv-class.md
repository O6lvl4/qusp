# Progress display を uv 級に揃える

**Phase 5 (Hospitality Parity)。**

## なぜ

現状 qusp の install / sync の progress 表示は backend ごとに微妙に
バラついてる。spinner はあるが ETA は無く、"downloaded N of M bytes"
みたいな byte-level progress も無い。`qusp install` で 200MB の GHC を
落とすときに「進んでるのか分からない」体験が出る (haskell e2e で実観察)。

uv は Python の `uv pip install` で「downloaded N of M」+ ETA を
妥協なく出してて、これが体感の肝。これを qusp の全 backend で揃える。

## 設計案

### Progress reporter trait

```rust
// HttpFetcher と同じレイヤで progress を effect として扱う
pub trait ProgressReporter: Send + Sync {
    fn start(&self, total_bytes: Option<u64>, label: &str) -> Box<dyn ProgressHandle>;
}
pub trait ProgressHandle: Send {
    fn advance(&mut self, n: u64);
    fn finish(&mut self);
}
```

`HttpFetcher::get_bytes` の中で reporter を回す。production 実装は
indicatif の ProgressBar。test 時は no-op で counter だけ取る Mock 版。

### Layout 統一

```
$ qusp install
[1/3] downloading scala 3.8.3 ...... ━━━━━━━━━━━━━━━━━━ 40MB/74MB · 18MB/s · 00:01
[2/3] downloading clojure 1.12.4.1618 ━━━━━━━━━━━━━━━━━ 15MB/15MB · finished
[3/3] downloading lua 5.4.7 ....... ━━━━━━━━━━━━━━━━━━━ 374KB/374KB · finished
✓ scala 3.8.3, clojure 1.12.4.1618, lua 5.4.7 installed in 4.2s
```

並列 install の場合は per-backend に行を分けてマルチライン更新
(indicatif の MultiProgress)。

### Build 系の進捗

Lua の `make`、Haskell の `ghcup install ghc` のような subprocess は
stdout を pipe して spinner で「elapsed time + 最後の log line」表示。

```
[2/3] building lua 5.4.7 (~5s) ....... ⠧ 02s · "gcc -O2 -Wall -Wextra ..."
```

uv の build の見せ方 (`Building wheel for X (pyproject.toml)`) と同じ
「進んでる感のある一行」が出る。

## 設計上の悩み

- **Backend trait の affirm 化**: 現状の Backend::install は HttpFetcher
  だけ受け取る。ProgressReporter も第二の effect として渡すと、trait
  signature が太くなる。effect bag を 1 つの struct にまとめる選択肢あり
  (`Effects { http, progress, ... }` を渡す)。
- **TTY 検出**: 非 tty (CI / pipe / `qusp install >file`) では progress
  bar を出さず、line-based log に fallback。indicatif は標準で対応済。
- **Quiet mode (`-q`)**: 既に `set_quiet(true)` がある。reporter が
  quiet なら no-op。

## 非ゴール

- Plot 風 ASCII art (uv は使ってないしオーバーキル)
- Color theme カスタマイズ (将来必要なら別)

## 実装ステップ

1. `crates/qusp-core/src/effects/progress.rs` ─ trait + indicatif 実装 +
   no-op 実装
2. `Backend::install` の signature に `&dyn ProgressReporter` 追加 (or
   effect bag に格上げ)
3. 各 backend の `http.get_bytes` 呼び出しを progress reporting 経由に
4. orchestrator の install_toolchains を MultiProgress に対応
5. spawn_blocking 系 (Lua make, ghcup install) は subprocess の
   stdout を tail して reporter に流す
