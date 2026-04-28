# Audit-Driven Full Migration Completion

**Shipped:** v0.14.0
**Tag:** v0.14.0

ユーザー指示「完全移行できてるか確認しよう」で実施した監査の結果、
2 種類の漏れを発見・修正:

1. **go.rs / ruby.rs に局所 reqwest::Client 構築が残っていた** (gv-core / rv-core が
   `&reqwest::Client` を直接要求するため)。
   → `HttpFetcher::as_reqwest_client(&self) -> Option<&reqwest::Client>` を trait に追加。
   `LiveHttp` が `Some(&self.client)` を返す。
   go.rs / ruby.rs は `require_reqwest(http)?` ヘルパー経由で取り出す。

2. **5 backends に boilerplate `install_tool` overrides** (`bail!` のみ)。
   → `Backend::install_tool` / `resolve_tool` に default impl 追加、override 削除。

最終監査:
- 局所 client 構築: 0
- 旧 `crate::http` 参照: 0
- `_http` (param 明示無視): 9 (全部「HTTP 不要メソッド」、bail or spawn_blocking 経路)

migration 完全達成。
