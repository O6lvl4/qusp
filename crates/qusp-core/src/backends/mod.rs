//! Built-in backends. Each is a native Rust implementation owning its
//! download / verify / install logic. No subprocess wrappers around
//! competing version managers.

pub mod bun;
pub mod deno;
pub mod go;
pub mod java;
pub mod node;
pub mod python;
pub mod ruby;
pub mod rust;
