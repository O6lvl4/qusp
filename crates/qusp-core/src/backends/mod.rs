//! Built-in backends. v0.0.1 ships Go (via `gv`) and Python (via `uv`)
//! as subprocess wrappers — proves the multi-language manifest end-to-end
//! while we incubate the deeper native backends in later releases.

pub mod go;
pub mod python;
