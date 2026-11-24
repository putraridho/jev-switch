//! The one place that waits for a wall-clock instant (DESIGN 7.1). The types are frozen;
//! T08 writes the bodies of `SystemTimer` and `FakeTimer`.

use jev_usage_core::time::UnixMillis;
use std::future::Future;
use std::pin::Pin;

/// Why a wait ended.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Wake {
    /// The deadline was reached.
    Due,
    /// The wall clock was set; every deadline must be recomputed.
    ClockChanged,
}

/// The future a wait returns.
pub type TimerFuture<'a> = Pin<Box<dyn Future<Output = Wake> + Send + 'a>>;

/// Waits for a wall-clock instant, so a deadline is still right after a suspend.
pub trait WallTimer: Send + Sync {
    /// Resolves when the wall clock reaches `deadline`, or when it is set.
    fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_>;
}

/// The real timer: an absolute `CLOCK_REALTIME` timerfd with cancel-on-set, in tokio's
/// `AsyncFd`.
#[derive(Debug, Default)]
pub struct SystemTimer {
    _private: (),
}

impl SystemTimer {
    /// A timer on the system's wall clock.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl WallTimer for SystemTimer {
    fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_> {
        let _ = deadline;
        Box::pin(std::future::pending())
    }
}

#[cfg(feature = "test-support")]
pub use fake::FakeTimer;

#[cfg(feature = "test-support")]
mod fake {
    use super::{TimerFuture, WallTimer};
    use crate::clock::FakeClock;
    use jev_usage_core::time::UnixMillis;
    use std::sync::Arc;

    /// Resolves when its `FakeClock` reaches the deadline (feature `test-support`).
    #[derive(Debug)]
    pub struct FakeTimer {
        _clock: Arc<FakeClock>,
    }

    impl FakeTimer {
        /// A timer that follows `clock`.
        pub fn new(clock: Arc<FakeClock>) -> Arc<Self> {
            Arc::new(Self { _clock: clock })
        }
    }

    impl WallTimer for FakeTimer {
        fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_> {
            let _ = deadline;
            Box::pin(std::future::pending())
        }
    }
}
