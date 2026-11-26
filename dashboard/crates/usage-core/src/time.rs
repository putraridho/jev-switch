//! Instants and the one source of "now" (DESIGN 4.1, 7.8). All instants are UTC epoch
//! integers end to end.

use std::time::Duration;

/// Milliseconds since the Unix epoch, UTC.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct UnixMillis(pub u64);

/// Seconds since the Unix epoch, UTC.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct UnixSeconds(pub u64);

impl UnixMillis {
    /// The same instant in whole seconds (rounded down).
    pub fn seconds(self) -> UnixSeconds {
        UnixSeconds(self.0 / 1000)
    }

    /// The time from `earlier` to `self`; zero when `earlier` is not earlier (clock skew).
    pub fn saturating_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

/// The only source of "now". Nothing in core or io calls `SystemTime` directly.
pub trait Clock: Send + Sync {
    /// The current wall-clock instant.
    fn now(&self) -> UnixMillis;
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

    #[test]
    fn seconds_round_down() {
        assert_eq!(UnixMillis(0).seconds(), UnixSeconds(0));
        assert_eq!(UnixMillis(999).seconds(), UnixSeconds(0));
        assert_eq!(UnixMillis(1000).seconds(), UnixSeconds(1));
        assert_eq!(
            UnixMillis(1_794_898_800_999).seconds(),
            UnixSeconds(1_794_898_800)
        );
        assert_eq!(UnixMillis(u64::MAX).seconds(), UnixSeconds(u64::MAX / 1000));
    }

    #[test]
    fn saturating_since_never_underflows() {
        assert_eq!(
            UnixMillis(5_000).saturating_since(UnixMillis(2_000)),
            Duration::from_secs(3)
        );
        assert_eq!(
            UnixMillis(2_000).saturating_since(UnixMillis(5_000)),
            Duration::ZERO
        );
        assert_eq!(
            UnixMillis(7).saturating_since(UnixMillis(7)),
            Duration::ZERO
        );
        assert_eq!(
            UnixMillis(u64::MAX).saturating_since(UnixMillis(0)),
            Duration::from_millis(u64::MAX)
        );
    }

    #[test]
    fn instants_order_by_value() {
        assert!(UnixMillis(1) < UnixMillis(2));
        assert!(UnixSeconds(9) > UnixSeconds(3));
    }

    struct Fixed(u64);
    impl Clock for Fixed {
        fn now(&self) -> UnixMillis {
            UnixMillis(self.0)
        }
    }

    #[test]
    fn clock_is_object_safe() {
        let clock: Box<dyn Clock> = Box::new(Fixed(42));
        assert_eq!(clock.now(), UnixMillis(42));
    }
}
