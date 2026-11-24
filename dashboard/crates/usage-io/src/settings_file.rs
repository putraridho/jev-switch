//! The settings file (DESIGN 7.4): `settings.json`, private and written atomically. The
//! type is frozen; T06 writes the bodies.

use jev_usage_core::port::{SettingsStore, StoreError};
use jev_usage_core::settings::Settings;
use std::path::PathBuf;

/// Settings as one JSON file. A missing or invalid file yields defaults.
#[derive(Debug)]
pub struct JsonSettingsStore {
    _path: PathBuf,
}

impl JsonSettingsStore {
    /// A store for the file at `path`; nothing is read yet.
    pub fn new(path: PathBuf) -> Self {
        Self { _path: path }
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Settings {
        Settings::defaults()
    }

    fn save(&self, s: &Settings) -> Result<(), StoreError> {
        let _ = s;
        Err(StoreError::Unavailable)
    }
}
