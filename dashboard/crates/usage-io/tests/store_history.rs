//! SQLite history integration tests.
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
use jev_usage_core::meter::{
    LimitStatus, Meter, MeterId, Meters, Reading, WindowKind, WindowReading,
};
use jev_usage_core::port::HistoryStore;
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::quantity::Percent;
use jev_usage_core::time::{UnixMillis, UnixSeconds};
use jev_usage_io::store::{MetaKey, SqliteStore};
fn reading(at: u64, value: u32) -> Reading {
    Reading {
        meters: Meters::new(vec![Meter::Window(WindowReading {
            kind: WindowKind::FiveHour,
            used: Percent::from_basis_points(value),
            resets_at: None,
            status: LimitStatus::Allowed,
        })])
        .unwrap(),
        plan: None,
        observed_at: UnixMillis(at * 1000),
    }
}
#[test]
fn write_rule_and_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("history.db")).unwrap();
    let values = [
        reading(1000, 1),
        reading(1100, 1),
        reading(1000, 2),
        reading(999, 2),
        reading(1200, 2),
        reading(2100, 2),
    ];
    assert_eq!(store.record_all(ProviderKey::Claude, &values).unwrap(), 3);
    let data = store
        .series(
            ProviderKey::Claude,
            &MeterId::Window(WindowKind::FiveHour),
            UnixSeconds(0),
            UnixSeconds(3000),
            100,
        )
        .unwrap();
    assert_eq!(
        data.points
            .iter()
            .map(|p| (p.at.0, p.value))
            .collect::<Vec<_>>(),
        vec![(1000, 1), (1200, 2), (2100, 2)]
    );
    store.meta_set(MetaKey::WindowWidth, 42).unwrap();
    assert_eq!(store.meta_get(MetaKey::WindowWidth).unwrap(), Some(42));
}
#[test]
fn corrupt_database_is_preserved_and_recreated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    std::fs::write(&path, b"not a sqlite database").unwrap();
    let store = SqliteStore::open(&path).unwrap();
    store
        .record(ProviderKey::Claude, &reading(1000, 1))
        .unwrap();
    assert_eq!(store.stats().unwrap().samples, 1);
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("history.db.corrupt-")));
}
#[test]
fn prune_rolls_up_old_samples() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(&dir.path().join("history.db")).unwrap();
    store
        .record_all(
            ProviderKey::Claude,
            &[reading(3600, 1), reading(3700, 3), reading(8_000_000, 2)],
        )
        .unwrap();
    store.prune(UnixSeconds(8_000_100)).unwrap();
    assert_eq!(store.stats().unwrap().samples, 1);
    let data = store
        .series(
            ProviderKey::Claude,
            &MeterId::Window(WindowKind::FiveHour),
            UnixSeconds(0),
            UnixSeconds(8_000_100),
            100,
        )
        .unwrap();
    assert_eq!(data.points.iter().map(|p| p.lo).min(), Some(1));
    assert_eq!(data.points.iter().map(|p| p.hi).max(), Some(3));
}
