# PHP

**優先度:** Phase 4 (2.x+)

## 設計

- php.net は source tarball + composer-style prebuilt は無い
- `phpenv` / `phpbrew` は source build
- prebuilt あるのは Linux distro 経由 (apt / yum)、macOS 経由 (brew)
- "qusp が build する" は ruby-build 同様 spawn_blocking パスになる

## 難所

- extension の管理 (curl / mbstring / openssl 等)
- 各 extension が configure オプションで切り替わる
- 「extensions も pin できる」が現実的なゴール
