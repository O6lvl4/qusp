# Network resilience + retry

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** A7 ─ "Network 不安定時の挙動"

## なぜ

実測 (audit 2026-04-28) で qusp が **transient parse error をユーザに直接出してる**:

```
$ qusp install
✗ python: parse python-build-standalone release index: EOF while parsing a value at line 1 column 0
```

これは python-build-standalone API の一時的な空応答 (CDN cache miss / rate-limit / network blip) だが、qusp の `LiveHttp` には retry layer が無いため失敗が即 user に到達する。

uv は libcurl ベースの retry を持ってて、同じ条件下で silent に retry → 成功して見せる。何回失敗してもユーザは気づかない。

audit シナリオで qusp は `qusp x ./hello.py` の最初の試行で常に失敗、retry した 2 回目で成功した。**毎回 fresh-machine の最初の体験が壊れてる** ことになる。これは hospitality 上クリティカル。

## 設計案

### A. HttpFetcher trait に retry layer

最も自然な場所は `LiveHttp` 内部:

```rust
// crates/qusp-core/src/effects/http.rs
const MAX_RETRIES: u32 = 5;
const BASE_DELAY_MS: u64 = 200;

impl LiveHttp {
    async fn get_text_with_retry(&self, url: &str) -> Result<String> {
        let mut delay = BASE_DELAY_MS;
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            match self.client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body = resp.text().await?;
                    if body.is_empty() && attempt < MAX_RETRIES - 1 {
                        // Empty body == transient. Retry.
                        last_err = Some(anyhow!("empty body from {url}"));
                    } else {
                        return Ok(body);
                    }
                }
                Ok(resp) if is_retryable_status(resp.status()) => {
                    last_err = Some(anyhow!("status {} from {url}", resp.status()));
                }
                Ok(resp) => {
                    // Non-retryable status (404, 401, etc.)
                    return Err(anyhow!("status {} from {url}", resp.status()));
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    last_err = Some(anyhow!("network error: {e}"));
                }
                Err(e) => return Err(e.into()),
            }
            tokio::time::sleep(Duration::from_millis(delay)).await;
            delay *= 2; // Exponential backoff
        }
        Err(last_err.unwrap_or_else(|| anyhow!("unknown retry exhaustion")))
    }
}

fn is_retryable_status(s: StatusCode) -> bool {
    s.is_server_error() || s == StatusCode::TOO_MANY_REQUESTS
}
```

### B. Empty-body も transient として扱う

audit で観察した EOF parse error は **HTTP 200 + empty body** が原因。これを retryable に分類しないと A だけでは効かない。`LiveHttp` の応答 body が `application/json` 期待で空文字列なら 1 度 retry する判定を入れる。

### C. User-visible behaviour

通常時: silent retry (uv 同形)。
最終失敗時: 既存の error context に "(retried N times)" を append。

```
error: parse python-build-standalone release index: EOF while parsing... (retried 5 times in 1.5s)
```

`-v` flag で各 retry attempt を log 出力。

## 設計上の悩み

- **Mock 側の retry**: `MockHttp` は test 用なので retry すると test
  semantics が変わる。Mock は retry 0、Live のみ retry の方針。
- **Idempotency**: retry できるのは GET と sha-verified bytes に限る。
  POST / state-changing は対象外 (qusp は basically GET のみなので
  問題なし)。
- **Timeout の階層**: per-request timeout vs total budget。uv は
  per-request 10s + total 60s 程度。qusp も同様の budget を導入。

## 非ゴール

- Backend ごとの custom retry policy (一律で十分)
- Connection pooling 高度化 (reqwest が既に持ってる)

## 実装ステップ

1. `crates/qusp-core/src/effects/http.rs` に retry helpers
2. `LiveHttp::get_text` / `get_bytes` / `get_text_authenticated` を retry 経由に
3. unit test (MockHttp で transient → retry → success のシナリオ)
4. e2e で flaky test を 100 回 loop して 0 failure を確認 (CI nightly で別 job)
