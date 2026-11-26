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

/// The real timer: an absolute `CLOCK_REALTIME` timerfd with cancel-on-set, waited on a
/// blocking thread (tokio's `AsyncFd` is unsound/deprecated in the pinned tokio, so the
/// timerfd is read with a plain blocking `read`).
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
        let secs = i64::try_from(deadline.0 / 1000).unwrap_or(i64::MAX);
        let nanos = i64::try_from((deadline.0 % 1000).saturating_mul(1_000_000)).unwrap_or(0);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use rustix::time::{
                    timerfd_create, timerfd_settime, Itimerspec, TimerfdClockId, TimerfdFlags,
                    TimerfdTimerFlags, Timespec,
                };
                let Ok(fd) = timerfd_create(TimerfdClockId::Realtime, TimerfdFlags::CLOEXEC) else {
                    return Wake::ClockChanged;
                };
                let spec = Itimerspec {
                    it_interval: Timespec {
                        tv_sec: 0,
                        tv_nsec: 0,
                    },
                    it_value: Timespec {
                        tv_sec: secs,
                        tv_nsec: nanos,
                    },
                };
                if timerfd_settime(
                    &fd,
                    TimerfdTimerFlags::ABSTIME | TimerfdTimerFlags::CANCEL_ON_SET,
                    &spec,
                )
                .is_err()
                {
                    return Wake::ClockChanged;
                }
                let mut buf = [0_u8; 8];
                match rustix::io::read(&fd, &mut buf) {
                    Ok(_) => Wake::Due,
                    Err(e) if e == rustix::io::Errno::CANCELED => Wake::ClockChanged,
                    Err(_) => Wake::ClockChanged,
                }
            })
            .await
            .unwrap_or(Wake::ClockChanged)
        })
    }
}

#[cfg(feature = "test-support")]
pub use fake::FakeTimer;

#[cfg(feature = "test-support")]
mod fake {
    use super::{TimerFuture, Wake, WallTimer};
    use crate::clock::{ClockMove, FakeClock};
    use jev_usage_core::time::{Clock, UnixMillis};
    use std::sync::Arc;

    /// Resolves when its `FakeClock` reaches the deadline (feature `test-support`).
    #[derive(Debug)]
    pub struct FakeTimer {
        clock: Arc<FakeClock>,
    }

    impl FakeTimer {
        /// A timer that follows `clock`.
        pub fn new(clock: Arc<FakeClock>) -> Arc<Self> {
            Arc::new(Self { clock })
        }
    }

    impl WallTimer for FakeTimer {
        fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_> {
            let clock = Arc::clone(&self.clock);
            Box::pin(async move {
                let mut moves = clock.moves();
                loop {
                    if clock.now() >= deadline {
                        return Wake::Due;
                    }
                    match moves.changed().await {
                        Ok(()) => {
                            if clock.now() >= deadline {
                                return Wake::Due;
                            }
                            if *moves.borrow() == ClockMove::Set {
                                return Wake::ClockChanged;
                            }
                        }
                        Err(_) => return Wake::ClockChanged,
                    }
                }
            })
        }
    }
}
