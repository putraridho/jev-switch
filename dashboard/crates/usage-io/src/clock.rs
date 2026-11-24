//! The wall clock (DESIGN 4.1, 7.8): the only place that asks the system for "now".

use jev_usage_core::time::{Clock, UnixMillis};
use std::time::{SystemTime, UNIX_EPOCH};

/// The system's wall clock.
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UnixMillis {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        UnixMillis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(feature = "test-support")]
pub use fake::FakeClock;

#[cfg(feature = "test-support")]
mod fake {
    use jev_usage_core::time::{Clock, UnixMillis};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// A clock tests set and advance by hand (feature `test-support`).
    #[derive(Debug)]
    pub struct FakeClock(AtomicU64);

    impl FakeClock {
        /// A clock that reads `ms` (epoch milliseconds).
        pub fn at(ms: u64) -> Arc<Self> {
            Arc::new(Self(AtomicU64::new(ms)))
        }

        /// Moves the clock forward by `d` (saturating).
        pub fn advance(&self, d: Duration) {
            let step = u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
            let _ = self
                .0
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                    Some(now.saturating_add(step))
                });
        }

        /// Sets the clock to `ms`; it may move backwards.
        pub fn set(&self, ms: u64) {
            self.0.store(ms, Ordering::SeqCst);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> UnixMillis {
            UnixMillis(self.0.load(Ordering::SeqCst))
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn system_clock_is_after_2025_and_does_not_go_back_within_a_call_pair() {
        let first = SystemClock.now();
        let second = SystemClock.now();
        assert!(first.0 > 1_735_689_600_000, "{first:?}");
        assert!(second.0 + 60_000 > first.0);
    }

    #[test]
    fn fake_clock_sets_and_advances() {
        let clock = FakeClock::at(1_000);
        assert_eq!(clock.now(), UnixMillis(1_000));
        clock.advance(Duration::from_millis(250));
        assert_eq!(clock.now(), UnixMillis(1_250));
        clock.set(40);
        assert_eq!(clock.now(), UnixMillis(40));
        clock.advance(Duration::MAX);
        assert_eq!(clock.now(), UnixMillis(u64::MAX));
        let shared: std::sync::Arc<dyn Clock> = clock;
        assert_eq!(shared.now(), UnixMillis(u64::MAX));
    }
}
