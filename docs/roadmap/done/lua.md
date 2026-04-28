# Lua

**Shipped:** v0.22.0
**Tag:** v0.22.0
**Phase 4 第八弾。Source-build パターンの初投入 (PHP/R/Erlang/OCaml の雛形)。LuaJIT は別 backend として保留。**

## なぜ source build に踏み込んだか

Phase 4 ここまで (Zig〜Clojure) は全て prebuilt distribution を pull
してきた。Lua は単一 ~370 KB ソース tarball + 5–10 秒 `make` で済む
のと、PUC-Rio が per-host バイナリを出してないので、source build が
**path of least resistance**。`ruby-build` 風の外部 dispatcher も
要らない (Lua の Makefile は autotools 不要、`make <plat> && make install`
のみ)。

このパターンは Phase 4 の残り source-build 系 (PHP/R/Elixir+Erlang/OCaml)
の雛形となる:
- `tokio::task::spawn_blocking` で sync `make` を tokio reactor から退避
- 検証フェーズ → 展開フェーズ → ビルドフェーズ → 配置フェーズの分離
- temp staging への逃避パターン (Application Support space trap 対策、後述)

## 設計

- **Source:** `https://www.lua.org/ftp/lua-<v>.tar.gz`
- **Verification:** **hardcoded SHA256 table** (lua.org は per-version
  sidecar を出してない、`download.html` に inline で current のみ)。
  qusp release prep 時に手動 `shasum -a 256` で table 更新。
- **Build:** `make <plat>` (autotools 不要、~30 .c) → `make install
  INSTALL_TOP=<staging>` (no autoconf / configure)
- **plat 検出:** `macosx`, `linux`, `bsd` (FreeBSD/NetBSD/OpenBSD)。
  `guess` ターゲットには頼らない (heuristic 失敗例があるため明示)。
- **Layout post-install:**
  ```
  <prefix>/bin/{lua, luac}
  <prefix>/include/{lua.h, luaconf.h, lualib.h, lauxlib.h, lua.hpp}
  <prefix>/lib/liblua.a              (static .a のみ、upstream の方針)
  <prefix>/man/man1/{lua.1, luac.1}
  <prefix>/share/lua/<v>/             (空、ユーザの Lua モジュール用)
  <prefix>/lib/lua/<v>/                (空、ユーザの C モジュール用)
  ```
- **Detect:** `.lua-version`
- **Tools:** empty。LuaRocks は別エコシステム。

## 検証戦略: hardcoded sha256 table

Pros:
- publisher が hash sidecar を出さないという事実を qusp release-prep
  で観測する責務として封じ込める (検証された事実だけが table に乗る)
- runtime の publisher trust 不要 (HTML scrape の脆弱性を持ち込まない)
- 一律 sha 検証ポリシーを破らない

Cons:
- 新 Lua version の対応に qusp release が必要。Lua の release 頻度
  (1–2/yr) なら許容範囲。security-relevant な版が出たら qusp も
  追従 release が必要なので副作用としては妥当。

table 内容 (v0.22.0 release prep 時点):
- 5.4.4, 5.4.5, 5.4.6, 5.4.7, 5.4.8, 5.5.0

## 落とし穴: macOS Application Support space trap (3 度目!)

`make install INSTALL_TOP=<path>` の `<path>` に space が含まれると、
Lua の Makefile の install: recipe で

  cd src && install -p -m 0755 lua luac $(INSTALL_BIN)

が `$(INSTALL_BIN) = $(INSTALL_TOP)/bin` を **unquoted** で展開し、
shell で word-split し

  install: /var/folders/.../Application: Inappropriate file type or format

を出して死ぬ。Make-level での quoting も Makefile を直接 patch しない
限り効かない (この backend の哲学では upstream に手は入れない)。

修正: `mktemp_no_space("qusp-lua")` で `std::env::temp_dir()`
(macOS では `/var/folders/...`、空間なしが保証されてる) 配下に
staging dir を切り、そこに `make install INSTALL_TOP=<staging>` →
完成後に staging tree を qusp store の `prefix/` に rename (or
fallback の copy_tree)。これで Lua の Makefile を patch せずに
回避できる。

これで space trap は **3 ケース目**:
1. v0.18.0 Groovy: upstream `bin/startGroovy` の `JAVA_OPTS` 裸展開
2. v0.21.0 Clojure: 自前 sed 置換時に `install_dir=PREFIX` 裸代入
3. v0.22.0 Lua: upstream `Makefile install:` の `$(INSTALL_BIN)` 裸展開

memory: `project_qusp_macos_appsupport_space_trap.md` に記録済み。
PHP/R/Erlang/OCaml の source-build 系では同じ staging-redirect が
使えるので `mktemp_no_space` + `copy_tree` ヘルパは shared utility
化を Phase 4 後半で検討する価値あり (今は per-backend で良い)。

## list_remote 設計

未検証 version は不要に list せず、qusp の sha256 table と完全一致
させる:
- 検証できるバージョンしか install できないので、list にも出さない
- ユーザが古いバージョンを試して失敗しないよう前段で防ぐ

## 4 unit tests

- known_sha256 が 5.4.4–5.4.8 + 5.5.0 を網羅、64 桁小文字 hex を保つ
- known_sha256 が未登録 version (古い 5.3.6 / 仮想 5.5.1 / garbage / "") を reject
- lua_makefile_plat が macOS/Linux/BSD/Windows/Redox を網羅
- version_cmp が 5.5.0 > 5.4.8 > 5.4.7 > 5.4.5 > 5.3.6 を保つ

## Smoke-tested

- `qusp install lua 5.4.7` → fetch + sha256 verify (qusp curated) +
  `make macosx` (~5s) + `make install INSTALL_TOP=<temp>` + relocate
- `qusp run lua -v` → "Lua 5.4.7  Copyright (C) 1994-2024 ..."
- `qusp run lua hello.lua` → "hi from qusp-managed lua"
- `qusp run luac -v` → "Lua 5.4.7"

## 非ゴール

- LuaJIT (別 backend `luajit` として保留、Lua 5.1 互換だが完全に別物)
- LuaRocks (Lua の package manager、別 tool / curated 後検討)
- Neovim 同梱の Lua (Neovim の責務)
