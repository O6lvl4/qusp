# Backend Unit Tests for the Remaining 7 Backends

**優先度:** Phase 2 (1.x)
**前提:** Phase 3.5 で全 backend が `&dyn HttpFetcher` を取る

## 現状

- python.rs: 7 unit tests (PBS fuzzy match, sums lookup)
- rust.rs: 3 unit tests (channel manifest parse)
- node.rs: 0
- bun.rs: 0
- deno.rs: 0
- kotlin.rs: 0
- java.rs: 0
- go.rs: 0 (gv-core 経由なので qusp 側で testable な層が薄い)
- ruby.rs: 0 (rv-core 経由、同上)

## やること

7 backends で MockHttp ベースの test を追加。テストすべきこと (各 backend で):

1. URL 構築が正しい (version + triple → asset URL の組み立て)
2. SHA sums のパース (line lookup, file format edge cases)
3. response が空 / malformed の時のエラーメッセージ
4. version-resolution 系のロジック (Java の Foojay filter, Kotlin の prerelease 排除, Bun の canary skip など)

go と ruby は MockHttp で as_reqwest_client が None を返すので一部 path のみ test 可能。
