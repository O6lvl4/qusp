//! Shared HTTP client construction.
//!
//! qusp talks to many hosts: publisher CDNs (static.rust-lang.org,
//! nodejs.org, hashicorp), GitHub API, npm registry, Foojay disco, etc.
//! Two constants matter for every one of them:
//!
//! - **User-Agent**: identifies us in publisher logs (`qusp-python/0.8.1`).
//! - **GitHub auth**: hosts under `api.github.com` are aggressively rate-
//!   limited for anonymous requests (60/hour). When `GITHUB_TOKEN` is
//!   set in the environment we attach `Authorization: Bearer <token>`,
//!   lifting the limit to 5000/hour.
//!
//! **The header MUST NOT be applied globally**, because some CDN
//! origins (like static.rust-lang.org) return `400 Bad Request` when
//! they see an Authorization header they didn't expect. Callers attach
//! it explicitly via [`gh_auth`] only on requests that actually hit
//! GitHub.

use anyhow::Result;

/// Build a base reqwest client with the qusp user-agent. **No
/// Authorization header is attached.** Callers add auth per-request
/// for github.com endpoints via [`gh_auth`].
pub fn client(user_agent: &str) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(user_agent).build()?)
}

/// Attach `Authorization: Bearer $GITHUB_TOKEN` to a request iff the
/// env var is set. Use on every `api.github.com` request, and nothing
/// else. reqwest 0.12+ strips the header on cross-origin redirects, so
/// even when a release-asset URL on `api.github.com` redirects to
/// `objects.githubusercontent.com`, the token is not leaked.
pub fn gh_auth(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return req.bearer_auth(token);
        }
    }
    req
}
