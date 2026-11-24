//! Private settings persistence tests.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_usage_core::port::SettingsStore;
use jev_usage_core::settings::Settings;
use jev_usage_io::{fsutil, settings_file::JsonSettingsStore};
use std::os::unix::fs::PermissionsExt;

#[test]
fn private_settings_round_trip_and_invalid_recovery() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("private");
    fsutil::ensure_private_dir(&dir).unwrap();
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let path = dir.join("settings.json");
    let store = JsonSettingsStore::new(path.clone());
    let settings = Settings::defaults();
    store.save(&settings).unwrap();
    assert_eq!(store.load(), settings);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::write(&path, b"invalid").unwrap();
    assert_eq!(store.load(), settings);
    assert!(dir.join("settings.json.invalid").exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}
