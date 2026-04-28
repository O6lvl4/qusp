# DDD Phase 2 — Pure Plan / Typed Errors

**Shipped:** v0.11.0
**Tag:** v0.11.0

`domain::types` に `LanguageId` / `Version` / `Distribution` newtypes。
`domain::plan` に純粋関数 `plan_install_toolchains` / `plan_sync`。
- IO 一切無し、ms 単位でテスト可能。

`Orchestrator::install_toolchains` / `sync` は plan + execute の thin wrapper に。
新たに `execute_install_plans` / `execute_sync_plan` が effect 層として export。

4 unit tests (plan 生成シェイプを全件カバー)。
