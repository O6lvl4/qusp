//! qusp's filesystem paths come from anyv-core, app name `"qusp"`.

pub use anyv_core::paths::{ensure_dir, Paths};

use anyhow::Result;

pub fn discover() -> Result<Paths> {
    Paths::discover("qusp")
}
