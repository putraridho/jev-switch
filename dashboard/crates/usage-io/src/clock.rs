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
pub(crate) use fake::ClockMove;

#[cfg(feature = "test-support")]
mod fake {
    use jev_usage_core::time::{Clock, UnixMillis};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::watch;

    /// How the clock moved: `advance` only ever moves forward; `set` may move anywhere.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub(crate) enum ClockMove {
        /// The clock moved forward by a duration.
        Advanced,
        /// The clock was set to an absolute value (it may have moved backwards).
        Set,
    }

    /// A clock tests set and advance by hand (feature `test-support`).
    pub struct FakeClock {
        ms: AtomicU64,
        changed: watch::Sender<ClockMove>,
    }

    impl std::fmt::Debug for FakeClock {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_tuple("FakeClock")
                .field(&self.ms.load(Ordering::SeqCst))
                .finish()
        }
    }

    impl FakeClock {
        /// A clock that reads `ms` (epoch milliseconds).
        pub fn at(ms: u64) -> Arc<Self> {
            let (changed, _rx) = watch::channel(ClockMove::Advanced);
            Arc::new(Self {
                ms: AtomicU64::new(ms),
                changed,
            })
        }

        /// A receiver that sees every move of the clock (used by `FakeTimer`).
        pub(crate) fn moves(&self) -> watch::Receiver<ClockMove> {
            self.changed.subscribe()
        }

        /// Moves the clock forward by `d` (saturating).
        pub fn advance(&self, d: Duration) {
            let step = u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
            let _ = self
                .ms
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                    Some(now.saturating_add(step))
                });
            self.changed.send_replace(ClockMove::Advanced);
        }

        /// Sets the clock to `ms`; it may move backwards.
        pub fn set(&self, ms: u64) {
            self.ms.store(ms, Ordering::SeqCst);
            self.changed.send_replace(ClockMove::Set);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> UnixMillis {
            UnixMillis(self.ms.load(Ordering::SeqCst))
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
