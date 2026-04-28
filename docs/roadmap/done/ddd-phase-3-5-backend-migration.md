# DDD Phase 3.5 — Backend Body Migration

**Shipped:** v0.12.1 (python) → v0.13.0 (全 backend)
**Tags:** v0.12.1 / v0.13.0

各 backend の install / list_remote / resolve_tool / install_tool 内部の
`reqwest::Client` 直接構築を全部 trait 経由に置換。

純粋ヘルパー extract:
- `python::pick_pbs_asset` / `sums_and_asset_urls` / `parse_sums_line`
- `node::parse_shasums_line` (bun も共有)
- `rust::parse_channel_rust_version` / `parse_toolchain_channel`

レガシー `crate::http` 削除。

unit tests: 17/17 (3 mock + 4 plan + 7 python + 3 rust)。
