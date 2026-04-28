# LuaJIT

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (Lua 5.1 互換 JIT、source build)
**前提:** `make` + C compiler

## なぜ別 backend か

Lua 本体 (`[lua]` v0.22.0 出荷済み) と LuaJIT は API は近いが処理系
としては完全に別物。互換性プロファイル / 性能特性 / build script /
リリースサイクル / コミュニティが分離。同一 backend に詰めるのは
ユーザの version 解釈を破壊する。

## 設計

- **Source:** `https://github.com/LuaJIT/LuaJIT/archive/refs/tags/v<v>.tar.gz` か
  `https://luajit.org/download/LuaJIT-<v>.tar.gz`
- **Verification:** GitHub mirror なら `asset.digest`、luajit.org なら
  hardcoded sha256 table (Lua 本体と同じ戦略)
- **Build:** `make PREFIX=<staging> && make install PREFIX=<staging>`
  - Lua 本体と同じ "staging dir で no-space" パターンで Application Support trap 回避
- **Versions:** 2.1.0 stable (2024 release)、2.0.5 (legacy)
- **Layout:** `<prefix>/{bin/{luajit, luajit-<v>}, include/luajit-2.1/, lib/{libluajit-5.1.a, libluajit-5.1.so.2}, share/man/man1/luajit.1}`

## 設計上の悩み

- **Lua 本体との name collision**: `luajit` バイナリ名なので qusp としては
  `[lua]` と `[luajit]` を別 manifest section にして問題なし
- **Lua 5.1 API互換だが OpenResty 等は LuaJIT 必須**: 既存 ecosystem の
  実態を尊重して別 backend で出す方向

## 非ゴール

- Lua 本体 (v0.22.0 で出荷済み)
- LuaRocks (別 tool)
