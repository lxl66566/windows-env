//! In-memory [`EnvStore`] implementation for tests.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::store::{EnvStore, ValueKind};
use crate::Result;

/// [`EnvStore`] backed by a `HashMap`, so unit tests never touch the real
/// registry, the process-wide named mutex, or `WM_SETTINGCHANGE` broadcasts.
#[derive(Default)]
pub(crate) struct MockStore {
    inner: Mutex<HashMap<String, (String, ValueKind)>>,
}

impl MockStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a value without going through validation.
    pub fn seed(&self, var: &str, value: &str, kind: ValueKind) {
        self.set(var, value, kind).unwrap();
    }

    /// Read back the raw stored pair for assertions on the value type.
    pub fn raw(&self, var: &str) -> Option<(String, ValueKind)> {
        self.inner.lock().unwrap().get(var).cloned()
    }
}

impl EnvStore for MockStore {
    fn get(&self, var: &str) -> Result<Option<(String, ValueKind)>> {
        Ok(self.raw(var))
    }

    fn set(&self, var: &str, value: &str, kind: ValueKind) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(var.to_owned(), (value.to_owned(), kind));
        Ok(())
    }

    fn delete(&self, var: &str) -> Result<()> {
        self.inner.lock().unwrap().remove(var);
        Ok(())
    }
}
