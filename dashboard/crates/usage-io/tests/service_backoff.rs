//! Tests for the retry schedule.
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

use jev_usage_core::usage::Unavailable;
use jev_usage_io::service::Backoff;
use std::time::Duration;

#[test]
fn failure_backoff_caps_and_applies_jitter() {
    for (failures, minutes) in [(1, 1), (2, 2), (3, 4), (7, 60), (u32::MAX, 60)] {
        assert_eq!(
            Backoff::next_delay(failures, Unavailable::Timeout, 0.5),
            Duration::from_secs(minutes * 60)
        );
    }
    assert_eq!(
        Backoff::next_delay(1, Unavailable::Timeout, 0.0),
        Duration::from_secs(54)
    );
    assert_eq!(
        Backoff::next_delay(1, Unavailable::Timeout, 1.0),
        Duration::from_secs(66)
    );
    assert_eq!(
        Backoff::next_delay(1, Unavailable::RateLimited, 0.5),
        Duration::from_mins(15)
    );
    assert_eq!(
        Backoff::next_delay(1, Unavailable::Unauthorized, 0.5),
        Duration::from_mins(15)
    );
}
