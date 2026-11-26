//! Tests for history policy.
use jev_usage_core::history::{gap_for, range_bounds, reset_marks, should_store, StoredSample};
use jev_usage_core::ipc::HistoryRange;
use jev_usage_core::meter::WindowKind;
use jev_usage_core::time::UnixSeconds;
use std::time::Duration;

#[test]
fn history_heartbeat_and_order() {
    let last = StoredSample {
        at: UnixSeconds(1000),
        value: 5,
        resets_at: None,
    };
    let mut next = last;
    assert!(!should_store(Some(&last), &next));
    next.at = UnixSeconds(1899);
    assert!(!should_store(Some(&last), &next));
    next.at = UnixSeconds(1900);
    assert!(should_store(Some(&last), &next));
}

#[test]
fn history_ranges_and_marks_are_bounded() {
    assert_eq!(
        range_bounds(HistoryRange::Day, UnixSeconds(100)),
        (UnixSeconds(0), UnixSeconds(100))
    );
    assert_eq!(gap_for(Duration::from_mins(1)), Duration::from_mins(75));
    assert_eq!(
        reset_marks(
            &[UnixSeconds(100), UnixSeconds(101), UnixSeconds(9100)],
            &WindowKind::FiveHour
        ),
        vec![UnixSeconds(100), UnixSeconds(9100)]
    );
}
