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

#[async_trait]
pub trait HttpFetcher: Send + Sync {
    async fn get_text(&self, url: &str) -> Result<String>;
    async fn get_bytes(&self, url: &str) -> Result<Bytes>;
    async fn get_text_authenticated(&self, url: &str) -> Result<String>;
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

#[async_trait]
impl HttpFetcher for LiveHttp {
    async fn get_text(&self, url: &str) -> Result<String> {
        Ok(self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("response error for {url}"))?
            .text()
            .await?)
    }

    async fn get_bytes(&self, url: &str) -> Result<Bytes> {
        Ok(self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("response error for {url}"))?
            .bytes()
            .await?)
    }

    async fn get_text_authenticated(&self, url: &str) -> Result<String> {
        let req = self.client.get(url);
        let req = self.attach_gh_auth(req);
        Ok(req
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("response error for {url}"))?
            .text()
            .await?)
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
