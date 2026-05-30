//! `HttpFetcher` — explicit HTTP effect.
//!
//! The single trait that backends call when they need to talk to the
//! network. Production code uses [`LiveHttp`] (a thin wrapper around
//! `reqwest::Client`); tests inject a mock that returns canned bodies
//! so backend behavior can be verified without leaving the process.
//!
//! Two flavours of every method:
//! - the bare `get_*` calls — for publisher CDNs, npm registry, Foojay,
//!   etc. No Authorization header attached.
//! - the `_authenticated` variants — for `api.github.com` calls. Lift
//!   the rate limit from 60/hr (anonymous) to 5000/hr by reading
//!   `GITHUB_TOKEN` from the environment. reqwest 0.12+ strips the
//!   header on cross-origin redirects so the token never leaks to
//!   `objects.githubusercontent.com` on a release-asset redirect.

use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;

use super::progress::ProgressTask;

#[async_trait]
pub trait HttpFetcher: Send + Sync {
    async fn get_text(&self, url: &str) -> Result<String>;
    async fn get_bytes(&self, url: &str) -> Result<Bytes>;
    async fn get_text_authenticated(&self, url: &str) -> Result<String>;

    /// Streamed download with per-chunk progress callback. Default
    /// impl just calls `get_bytes` and reports the full size at the
    /// end (correct, but provides no real-time feedback). `LiveHttp`
    /// overrides for actual chunk-level reporting using
    /// `Content-Length` to set the bar total before the first byte.
    async fn get_bytes_streaming(&self, url: &str, task: &mut dyn ProgressTask) -> Result<Bytes> {
        let bytes = self.get_bytes(url).await?;
        task.set_total(bytes.len() as u64);
        task.advance(bytes.len() as u64);
        Ok(bytes)
    }

    /// Escape hatch: backends that pass through to libraries which
    /// still want a raw `reqwest::Client` (gv-core / rv-core) call
    /// this. `LiveHttp` returns its inner client. Mocks return `None`,
    /// which is the right shape — those backends can't be unit-tested
    /// with MockHttp until upstream learns about HttpFetcher.
    fn as_reqwest_client(&self) -> Option<&reqwest::Client> {
        None
    }
}

/// Production implementation backed by reqwest.
pub struct LiveHttp {
    client: reqwest::Client,
}

impl LiveHttp {
    /// Build a LiveHttp with the given User-Agent. The User-Agent is
    /// what shows up in publisher access logs (`qusp/0.11.0`).
    pub fn new(user_agent: &str) -> Result<Self> {
        let client = reqwest::Client::builder().user_agent(user_agent).build()?;
        Ok(Self { client })
    }

    /// Direct access to the inner reqwest client when something needs
    /// a streaming response or other reqwest-specific knob. Avoid in
    /// new code — prefer extending the trait.
    pub fn raw(&self) -> &reqwest::Client {
        &self.client
    }

    fn attach_gh_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            let token = token.trim();
            if !token.is_empty() {
                return req.bearer_auth(token);
            }
        }
        req
    }
}

/// Max total attempts (1 initial + retries) before giving up on a
/// transient failure.
const MAX_HTTP_ATTEMPTS: u32 = 4;
/// Base backoff; doubles each retry (250ms → 500ms → 1s).
const HTTP_BACKOFF_BASE_MS: u64 = 250;

/// Whether a request outcome is worth retrying. `status` is the HTTP
/// status if a response arrived; `transport` is true for
/// connect/timeout/body/decode-level errors (no status). We retry
/// transport blips and 5xx / 429 — never other 4xx, which are real
/// answers (a 404/401 won't get better by asking again).
fn is_retryable(status: Option<reqwest::StatusCode>, transport: bool) -> bool {
    match status {
        Some(s) => s.is_server_error() || s == reqwest::StatusCode::TOO_MANY_REQUESTS,
        None => transport,
    }
}

fn http_backoff(attempt: u32) -> std::time::Duration {
    let shift = attempt.saturating_sub(1).min(4);
    std::time::Duration::from_millis(HTTP_BACKOFF_BASE_MS * (1u64 << shift))
}

/// reqwest errors that represent a flaky link rather than a definitive
/// answer: connection refused/reset, timeouts, and partial/truncated
/// bodies (the "end of file before message length reached" class).
fn reqwest_is_transient(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_decode()
}

impl LiveHttp {
    /// Run a GET — built by `make_req`, body read by `read` — retrying
    /// transient failures with backoff. Both the request *and* the body
    /// read are inside the retried block, so a connection dropped
    /// mid-body (the flaky-CI failure mode) gets another shot instead of
    /// surfacing raw.
    async fn run_with_retry<T, MakeReq, Read, ReadFut>(
        &self,
        url: &str,
        make_req: MakeReq,
        read: Read,
    ) -> Result<T>
    where
        MakeReq: Fn() -> reqwest::RequestBuilder,
        Read: Fn(reqwest::Response) -> ReadFut,
        ReadFut: std::future::Future<Output = reqwest::Result<T>>,
    {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let outcome: reqwest::Result<T> = async {
                let resp = make_req().send().await?.error_for_status()?;
                read(resp).await
            }
            .await;
            match outcome {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let retry = attempt < MAX_HTTP_ATTEMPTS
                        && is_retryable(e.status(), reqwest_is_transient(&e));
                    if !retry {
                        return Err(e)
                            .with_context(|| format!("GET {url} (after {attempt} attempt(s))"));
                    }
                    tokio::time::sleep(http_backoff(attempt)).await;
                }
            }
        }
    }
}

#[async_trait]
impl HttpFetcher for LiveHttp {
    async fn get_text(&self, url: &str) -> Result<String> {
        self.run_with_retry(url, || self.client.get(url), |r| r.text())
            .await
    }

    async fn get_bytes(&self, url: &str) -> Result<Bytes> {
        self.run_with_retry(url, || self.client.get(url), |r| r.bytes())
            .await
    }

    async fn get_bytes_streaming(&self, url: &str, task: &mut dyn ProgressTask) -> Result<Bytes> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let outcome: reqwest::Result<Bytes> = async {
                let resp = self.client.get(url).send().await?.error_for_status()?;
                if let Some(total) = resp.content_length() {
                    task.set_total(total);
                }
                let mut buf: Vec<u8> = Vec::new();
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    task.advance(chunk.len() as u64);
                    buf.extend_from_slice(&chunk);
                }
                Ok(Bytes::from(buf))
            }
            .await;
            match outcome {
                Ok(b) => return Ok(b),
                Err(e) => {
                    let retry = attempt < MAX_HTTP_ATTEMPTS
                        && is_retryable(e.status(), reqwest_is_transient(&e));
                    if !retry {
                        return Err(e).with_context(|| {
                            format!("download {url} (after {attempt} attempt(s))")
                        });
                    }
                    // A retried stream re-reads from the top; the progress
                    // bar may tick past 100% on the rare retry, which is
                    // cosmetic.
                    tokio::time::sleep(http_backoff(attempt)).await;
                }
            }
        }
    }

    async fn get_text_authenticated(&self, url: &str) -> Result<String> {
        self.run_with_retry(
            url,
            || self.attach_gh_auth(self.client.get(url)),
            |r| r.text(),
        )
        .await
    }

    fn as_reqwest_client(&self) -> Option<&reqwest::Client> {
        Some(&self.client)
    }
}

#[cfg(test)]
pub mod mock {
    //! Test double for [`HttpFetcher`].
    //!
    //! Build with `MockHttp::default().with_text(url, body)` /
    //! `with_bytes(url, body)`. Calling a URL that wasn't registered
    //! errors so missing-stub bugs surface loudly.

    use std::collections::HashMap;
    use std::sync::Mutex;

    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use bytes::Bytes;

    use super::HttpFetcher;

    #[derive(Default)]
    pub struct MockHttp {
        text: Mutex<HashMap<String, String>>,
        bytes: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MockHttp {
        pub fn with_text(self, url: impl Into<String>, body: impl Into<String>) -> Self {
            self.text.lock().unwrap().insert(url.into(), body.into());
            self
        }
        pub fn with_bytes(self, url: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
            self.bytes.lock().unwrap().insert(url.into(), body.into());
            self
        }
    }

    #[async_trait]
    impl HttpFetcher for MockHttp {
        async fn get_text(&self, url: &str) -> Result<String> {
            self.text
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| anyhow!("MockHttp: no text response for {url}"))
        }

        async fn get_bytes(&self, url: &str) -> Result<Bytes> {
            self.bytes
                .lock()
                .unwrap()
                .get(url)
                .map(|v| Bytes::from(v.clone()))
                .ok_or_else(|| anyhow!("MockHttp: no bytes response for {url}"))
        }

        async fn get_text_authenticated(&self, url: &str) -> Result<String> {
            self.get_text(url).await
        }
    }

    #[cfg(test)]
    mod self_tests {
        use super::*;

        #[tokio::test]
        async fn returns_text_when_registered() {
            let m = MockHttp::default().with_text("https://x.test/sums", "abc  asset.zip\n");
            assert_eq!(
                m.get_text("https://x.test/sums").await.unwrap(),
                "abc  asset.zip\n"
            );
        }

        #[tokio::test]
        async fn errors_on_unregistered_url() {
            let m = MockHttp::default();
            let err = m.get_text("https://x.test/missing").await.unwrap_err();
            assert!(err.to_string().contains("no text response"));
        }

        #[tokio::test]
        async fn authenticated_falls_through_to_text_in_mock() {
            // Mock doesn't differentiate auth vs anonymous — the production
            // LiveHttp does. This is the right shape for testing publisher
            // logic without baking auth assumptions into mocks.
            let m = MockHttp::default().with_text("https://api.github.com/x", r#"{"a":1}"#);
            assert!(m
                .get_text_authenticated("https://api.github.com/x")
                .await
                .unwrap()
                .contains("\"a\":1"));
        }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::{http_backoff, is_retryable, HTTP_BACKOFF_BASE_MS};
    use reqwest::StatusCode;

    #[test]
    fn transport_errors_retry_5xx_and_429_retry_other_4xx_dont() {
        // No status → transport blip: retry only when the transport flag is set.
        assert!(is_retryable(None, true));
        assert!(!is_retryable(None, false));
        // Server errors and rate-limit: retry.
        assert!(is_retryable(Some(StatusCode::INTERNAL_SERVER_ERROR), false));
        assert!(is_retryable(Some(StatusCode::BAD_GATEWAY), false));
        assert!(is_retryable(Some(StatusCode::TOO_MANY_REQUESTS), false));
        // Definitive client answers: never retry.
        assert!(!is_retryable(Some(StatusCode::NOT_FOUND), true));
        assert!(!is_retryable(Some(StatusCode::UNAUTHORIZED), false));
        assert!(!is_retryable(Some(StatusCode::OK), false));
    }

    #[test]
    fn backoff_grows_and_caps() {
        let ms = |a| http_backoff(a).as_millis() as u64;
        assert_eq!(ms(1), HTTP_BACKOFF_BASE_MS); // 250
        assert_eq!(ms(2), HTTP_BACKOFF_BASE_MS * 2); // 500
        assert_eq!(ms(3), HTTP_BACKOFF_BASE_MS * 4); // 1000
                                                     // Exponent caps so a high attempt count can't explode the delay.
        assert_eq!(ms(50), HTTP_BACKOFF_BASE_MS * 16);
    }
}
