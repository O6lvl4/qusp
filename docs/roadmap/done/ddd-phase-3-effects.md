# DDD Phase 3 — HttpFetcher Trait

**Shipped:** v0.12.0
**Tag:** v0.12.0

`crate::effects::HttpFetcher` trait — `get_text` / `get_bytes` / `get_text_authenticated`。
`LiveHttp` (production, reqwest wrapper) / `MockHttp` (tests).

`Backend` trait: `install` / `list_remote` / `resolve_tool` / `install_tool` が
`&dyn HttpFetcher` を取るように変更。

3 mock-http self-tests + 既存 4 plan tests = 7 unit tests。
