# Lua / LuaJIT

**優先度:** Phase 4 (2.x+)

## 設計

- Lua: lua.org の tarball (要 source build, autoconf 不要、make だけ)
- LuaJIT: luajit.org tarball, makefile build
- Source build なので spawn_blocking 必要、ruby-build 同様 "唯一の例外" カテゴリ
