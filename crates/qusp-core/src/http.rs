//! Shared HTTP client construction.
//!
//! qusp talks to many hosts (publisher CDNs, GitHub API, npm registry,
//! Foojay disco, …). Two constants matter for every one of them:
//!
//! - **User-Agent**: identifies us in publisher logs (`qusp-python/0.8.1`).
//! - **GitHub auth**: hosts under `api.github.com` are aggressively rate-
//!   limited for anonymous requests (60/hour). When `GITHUB_TOKEN` is set
//!   in the environment (which CI always provides), we attach a bearer
//!   token so the limit jumps to 5000/hour.
//!
//! reqwest strips the `Authorization` header on cross-origin redirects
//! by default, so the token is never leaked when GitHub redirects an
//! asset URL to `objects.githubusercontent.com` for the actual download.

use anyhow::Result;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

/// Build a reqwest client with the qusp user-agent and (when set)
/// `Authorization: Bearer $GITHUB_TOKEN`.
pub fn client(user_agent: &str) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(AUTHORIZATION, v);
            }
        }
    }
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .default_headers(headers)
        .build()?)
}
