//! In-process registry of language backends. Wires user-facing language
//! ids ("go", "python", …) to live `Box<dyn Backend>` instances.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::backend::Backend;

#[derive(Default)]
pub struct BackendRegistry {
    backends: BTreeMap<&'static str, Arc<dyn Backend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, b: Arc<dyn Backend>) {
        self.backends.insert(b.id(), b);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Backend>> {
        self.backends.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.backends.keys().copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Arc<dyn Backend>)> + '_ {
        self.backends.iter().map(|(k, v)| (*k, v.clone()))
    }
}
