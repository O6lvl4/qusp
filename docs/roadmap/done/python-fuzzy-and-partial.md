# Python Fuzzy Match + Partial-Success Install

**Shipped:** v0.8.1
**Tag:** v0.8.1

2 つの実バグ fix:

1. **`try_join_all` が並列 install を一発で殺す** — Python の typo で go/rust/bun も殺してた。
   `join_all` に切り替え、`InstallToolchainsResult { installed, failed }` で partial 報告。
2. **PBS が exact `3.13.0` を publish しなくなってた** — fuzzy match 追加。
   exact 試行 → fallback で `3.13.x` の latest patch を採用。
