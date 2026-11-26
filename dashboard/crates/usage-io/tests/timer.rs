//! The wall timer: SystemTimer (real timerfd) and FakeTimer (clock-following).

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

use jev_usage_core::time::UnixMillis;
use jev_usage_io::clock::FakeClock;
use jev_usage_io::timer::{FakeTimer, SystemTimer, Wake, WallTimer};
use std::time::Duration;

#[tokio::test]
async fn fake_timer_fires_when_the_clock_advances_past_the_deadline() {
    let clock = FakeClock::at(1_000);
    let timer = FakeTimer::new(clock.clone());
    let advance = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        clock.advance(Duration::from_secs(5));
    });
    let wake = timer.sleep_until(UnixMillis(5_000)).await;
    assert_eq!(wake, Wake::Due);
    advance.await.unwrap();
}

#[tokio::test]
async fn fake_timer_reports_clock_changed_when_set_backwards() {
    let clock = FakeClock::at(10_000);
    let timer = FakeTimer::new(clock.clone());
    let set = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        clock.set(5_000);
    });
    let wake = timer.sleep_until(UnixMillis(20_000)).await;
    assert_eq!(wake, Wake::ClockChanged);
    set.await.unwrap();
}

#[tokio::test]
async fn fake_timer_fires_at_once_for_a_past_deadline() {
    let clock = FakeClock::at(10_000);
    let timer = FakeTimer::new(clock);
    assert_eq!(timer.sleep_until(UnixMillis(5_000)).await, Wake::Due);
}

#[tokio::test]
async fn system_timer_fires_at_a_short_deadline_and_at_once_for_the_past() {
    let timer = SystemTimer::new();
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let past = timer
        .sleep_until(UnixMillis(now.saturating_sub(1_000)))
        .await;
    assert_eq!(past, Wake::Due);
    let soon = timer.sleep_until(UnixMillis(now + 50)).await;
    assert_eq!(soon, Wake::Due);
}
