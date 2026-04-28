//! Built-in backends. Each is a native Rust implementation owning its
//! download / verify / install logic. No subprocess wrappers around
//! competing version managers.

pub mod deno;
pub mod go;
pub mod node;
pub mod python;
pub mod ruby;
