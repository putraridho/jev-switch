//! The settings file (DESIGN 7.4): private and written atomically.

use crate::fsutil::write_private_atomic;
use jev_usage_core::port::{SettingsStore, StoreError};
use jev_usage_core::settings::Settings;
use std::path::PathBuf;

/// Settings as one JSON file. A missing or invalid file yields defaults.
#[derive(Debug)]
pub struct JsonSettingsStore {
    path: PathBuf,
}

impl JsonSettingsStore {
    /// A store for the file at `path`; nothing is read yet.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Settings {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                if let Ok(settings) = serde_json::from_slice::<Settings>(&bytes) {
                    settings.validated()
                } else {
                    let mut invalid = self.path.as_os_str().to_os_string();
                    invalid.push(".invalid");
                    let _ = std::fs::rename(&self.path, PathBuf::from(invalid));
                    Settings::defaults()
                }
            }
            Err(_) => Settings::defaults(),
        }
    }

    fn save(&self, s: &Settings) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(s).map_err(|_| StoreError::Unavailable)?;
        write_private_atomic(&self.path, &bytes).map_err(|_| StoreError::Unavailable)
    }
}
