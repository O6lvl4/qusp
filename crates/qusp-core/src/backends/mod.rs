//! Built-in backends. Each is a native Rust implementation owning its
//! download / verify / install logic. No subprocess wrappers around
//! competing version managers.

pub mod bun;
pub mod clojure;
pub mod crystal;
pub mod dart;
pub mod deno;
pub mod go;
pub mod groovy;
pub mod java;
pub mod julia;
pub mod kotlin;
pub mod node;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod scala;
pub mod zig;
