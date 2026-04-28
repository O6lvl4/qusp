# Lua / LuaJIT

**優先度:** Phase 4 (2.x+)
**難易度:** 低-中 (source build は単純、ただし spawn_blocking)

## なぜ

CLI/embedded scripting で根強い (nginx OpenResty, Redis, Neovim, AwesomeWM, …)。
Neovim plugin 開発で 2026 もメイン。

## 設計

### Lua (PUC-Rio)

- **Source:** `https://www.lua.org/ftp/lua-{version}.tar.gz`
- **Verification:** lua.org が published page で md5 + sha256 を出してる (HTML scrape か、人間が register した hash table を qusp が持つ)
- **Build:** `make {macosx|linux} INSTALL_TOP=<prefix>` — autotools 不要、5-10 秒で build。
- **Versions:** 5.1, 5.2, 5.3, 5.4 が並行 supported。互換性破壊あるので exact pin。
- **Layout:** `bin/{lua, luac}`, `include/`, `lib/{liblua.a}`, `share/man/`

### LuaJIT (separate backend)

Lua 5.1 互換の高速処理系。完全に別物。
- **Source:** `https://luajit.org/download/LuaJIT-{version}.tar.gz` or GitHub releases
- **Build:** `make install PREFIX=<prefix>`
- **Versions:** 2.1.0-beta3 が長らく最新だった、最近 2.1.0 stable リリース

## 設計上の悩み

- **make build なので spawn_blocking**: ruby-build / php-build / OTP build と同じ「唯一の例外」カテゴリ
- **sha256 が published page にしか無い**: 公式が hash sidecar を出してないので qusp 側で hardcoded table を持つか scrape するか。前者の方が安全 (publisher が hash を出さない事実は backend 設計の判断対象)
- **5.x の major switch**: 5.1 と 5.4 で言語仕様が違う。どちらをデフォルトにするか — exact pin 必須 (デフォルトなし) が筋

## 非ゴール

- LuaRocks (package manager) 管理。LuaRocks 自体は別 tool として curated 可能。
- Neovim 経由の Lua (Neovim 同梱) 管理。

## 実装ステップ

1. `crates/qusp-core/src/backends/lua.rs` (PUC-Rio)
2. `crates/qusp-core/src/backends/luajit.rs` (LuaJIT)
3. SHA256 hardcoded table (5.1 系: x, 5.2 系: y, ...) — qusp release 時に手動 audit
4. e2e/lua.sh
