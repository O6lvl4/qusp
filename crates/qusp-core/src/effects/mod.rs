//! Explicit effect traits.
//!
//! Phase 3 of the Functional-DDD migration. Backends used to construct
//! their own `reqwest::Client` and call `client.get(url)` inline,
//! interleaving HTTP with domain decisions. After Phase 3, the
//! Backend trait's IO methods take a `&dyn HttpFetcher` they don't
//! own — production wires in [`http::LiveHttp`], tests wire in a
//! mock that returns canned responses. Backend logic becomes
//! genuinely unit-testable.
//!
//! Filesystem effects intentionally stay on the standard library for
//! now: the surface (read_dir, write, symlink, set_permissions, …)
//! is large, and the kinds of tests we get the most leverage from
//! (URL construction, sha verification, version-fuzzy-match) don't
//! need a fake FS to be useful.

pub mod http;

pub use http::{HttpFetcher, LiveHttp};
