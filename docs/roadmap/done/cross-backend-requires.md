# Cross-Backend `Backend::requires`

**Shipped:** v0.5.0 (機構) → v0.9.0 (Kotlin で実証)

`Backend::requires(&self) -> &[&'static str]` を trait に追加。
default は空、Kotlin が `["java"]` を返す。
Orchestrator の `validate_requires()` が install 前に検証。
`[kotlin]` を pin して `[java]` が無いと、ダウンロード前に親切エラー。

cross-backend env merge は orchestrator が build_run_env で自動。
Kotlin backend は Java の存在を意識しない。
