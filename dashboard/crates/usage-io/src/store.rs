//! The SQLite history store (DESIGN 7.5). The types and constants are frozen; T06 writes
//! the bodies.

use jev_usage_core::alerts::AlertMemory;
use jev_usage_core::history::{SeriesData, StoreStats};
use jev_usage_core::meter::{MeterId, Reading};
use jev_usage_core::port::{HistoryStore, StoreError};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::UnixSeconds;
use std::path::Path;

/// The schema version this binary writes.
pub const SCHEMA_VERSION: i64 = 1;

/// The integers kept in the store's `meta` table.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum MetaKey {
    /// Set once the first-run backfill has finished.
    BackfillDone,
    /// When the last prune ran (epoch seconds).
    LastPruneAt,
    /// The window's last width.
    WindowWidth,
    /// The window's last height.
    WindowHeight,
}

/// One connection behind a mutex. Every method does file IO: callers use `spawn_blocking`.
#[derive(Debug)]
pub struct SqliteStore {
    _private: (),
}

impl SqliteStore {
    /// Opens (or creates, 0600) the database at `path`: pragmas, migration; a corrupt file
    /// is renamed aside and recreated.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let _ = path;
        Err(StoreError::Unavailable)
    }

    /// One integer of the `meta` table.
    pub fn meta_get(&self, key: MetaKey) -> Result<Option<i64>, StoreError> {
        let _ = key;
        Err(StoreError::Unavailable)
    }

    /// Sets one integer of the `meta` table.
    pub fn meta_set(&self, key: MetaKey, value: i64) -> Result<(), StoreError> {
        let _ = (key, value);
        Err(StoreError::Unavailable)
    }

    /// Rows stepped from SQLite by the last `series` call (feature `test-support`).
    #[cfg(feature = "test-support")]
    pub fn last_series_rows(&self) -> u64 {
        0
    }
}

impl HistoryStore for SqliteStore {
    fn record(&self, provider: ProviderKey, reading: &Reading) -> Result<(), StoreError> {
        let _ = (provider, reading);
        Err(StoreError::Unavailable)
    }

    fn record_all(&self, provider: ProviderKey, readings: &[Reading]) -> Result<u64, StoreError> {
        let _ = (provider, readings);
        Err(StoreError::Unavailable)
    }

    fn series(
        &self,
        provider: ProviderKey,
        meter: &MeterId,
        from: UnixSeconds,
        to: UnixSeconds,
        max_points: u16,
    ) -> Result<SeriesData, StoreError> {
        let _ = (provider, meter, from, to, max_points);
        Err(StoreError::Unavailable)
    }

    fn load_alerts(&self) -> Result<AlertMemory, StoreError> {
        Err(StoreError::Unavailable)
    }

    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError> {
        let _ = memory;
        Err(StoreError::Unavailable)
    }

    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError> {
        let _ = now;
        Err(StoreError::Unavailable)
    }

    fn stats(&self) -> Result<StoreStats, StoreError> {
        Err(StoreError::Unavailable)
    }
}
