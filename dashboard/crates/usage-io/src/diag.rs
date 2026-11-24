//! The one diagnostic channel (DESIGN 8.4). `DiagEvent` holds numbers and closed enums, so
//! there is no way to log a string; output is one line per event on stderr and nowhere else.

use jev_usage_core::port::{SourceId, StoreError};
use jev_usage_core::usage::Unavailable;
use std::io::Write;

/// Everything the app can say about itself.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum DiagEvent {
    /// A fetch succeeded.
    SourceOk {
        /// The source.
        source: SourceId,
        /// How long the fetch took, in milliseconds.
        took_ms: u32,
    },
    /// A fetch failed.
    SourceFailed {
        /// The source.
        source: SourceId,
        /// Why.
        reason: Unavailable,
    },
    /// A store call failed.
    StoreFailed {
        /// Why.
        error: StoreError,
    },
    /// The gateway log was rotated.
    LogRotated,
    /// The file watch was lost and is being re-armed.
    WatchLost,
    /// The file watch is in place.
    WatchArmed,
    /// The wall clock was set.
    ClockChanged,
    /// A login probe finished.
    ProbeDone {
        /// How long it took, in milliseconds.
        took_ms: u32,
    },
    /// The first-run backfill finished.
    BackfillDone {
        /// Samples stored.
        samples: u32,
    },
    /// A new web view showed its first data.
    WindowReady {
        /// Milliseconds from window creation to the first snapshot.
        took_ms: u32,
    },
    /// A notification could not be delivered.
    NotifyFailed,
    /// The autostart entry could not be written.
    AutostartFailed,
    /// The app started.
    Started,
    /// The app is shutting down.
    Stopping,
}

/// The event as its one line (without the newline): a fixed prefix, the event's name, then
/// `name=value` pairs of numbers and enum names.
pub fn line(e: DiagEvent) -> String {
    match e {
        DiagEvent::SourceOk { source, took_ms } => {
            format!("flashboard: SourceOk source={source:?} took_ms={took_ms}")
        }
        DiagEvent::SourceFailed { source, reason } => {
            format!("flashboard: SourceFailed source={source:?} reason={reason:?}")
        }
        DiagEvent::StoreFailed { error } => format!("flashboard: StoreFailed error={error:?}"),
        DiagEvent::ProbeDone { took_ms } => format!("flashboard: ProbeDone took_ms={took_ms}"),
        DiagEvent::BackfillDone { samples } => {
            format!("flashboard: BackfillDone samples={samples}")
        }
        DiagEvent::WindowReady { took_ms } => format!("flashboard: WindowReady took_ms={took_ms}"),
        DiagEvent::LogRotated
        | DiagEvent::WatchLost
        | DiagEvent::WatchArmed
        | DiagEvent::ClockChanged
        | DiagEvent::NotifyFailed
        | DiagEvent::AutostartFailed
        | DiagEvent::Started
        | DiagEvent::Stopping => format!("flashboard: {e:?}"),
    }
}

/// Writes the event to stderr as one line. A failed write is dropped: diagnostics never
/// fail the caller. While a test holds a [`capture`] guard the event is collected instead.
pub fn emit(e: DiagEvent) {
    #[cfg(feature = "test-support")]
    if capture::collect(e) {
        return;
    }
    let mut text = line(e);
    text.push('\n');
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}

#[cfg(feature = "test-support")]
pub use capture::{capture, CaptureGuard};

#[cfg(feature = "test-support")]
mod capture {
    use super::DiagEvent;
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// Held by one capturing test at a time.
    static TURN: Mutex<()> = Mutex::new(());
    /// The events of the capture in progress; `None` when nothing captures.
    static SINK: Mutex<Option<Vec<DiagEvent>>> = Mutex::new(None);

    fn sink() -> MutexGuard<'static, Option<Vec<DiagEvent>>> {
        SINK.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores `e` when a capture is in progress and says whether it did.
    pub(super) fn collect(e: DiagEvent) -> bool {
        match sink().as_mut() {
            Some(events) => {
                events.push(e);
                true
            }
            None => false,
        }
    }

    /// While it lives, `emit` collects instead of printing (feature `test-support`).
    pub struct CaptureGuard {
        _turn: MutexGuard<'static, ()>,
    }

    impl CaptureGuard {
        /// The events emitted since the capture began, in order.
        pub fn events(&self) -> Vec<DiagEvent> {
            sink().clone().unwrap_or_default()
        }
    }

    impl Drop for CaptureGuard {
        fn drop(&mut self) {
            *sink() = None;
        }
    }

    /// Collect instead of print (feature `test-support`). Process-wide; the guard holds a
    /// mutex, so tests that capture run one at a time and never see each other's events.
    pub fn capture() -> CaptureGuard {
        let turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        *sink() = Some(Vec::new());
        CaptureGuard { _turn: turn }
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

    fn every_event() -> Vec<DiagEvent> {
        vec![
            DiagEvent::SourceOk {
                source: SourceId::CodexAppServer,
                took_ms: 2750,
            },
            DiagEvent::SourceFailed {
                source: SourceId::DeepSeekBalance,
                reason: Unavailable::Unauthorized,
            },
            DiagEvent::StoreFailed {
                error: StoreError::Corrupt,
            },
            DiagEvent::LogRotated,
            DiagEvent::WatchLost,
            DiagEvent::WatchArmed,
            DiagEvent::ClockChanged,
            DiagEvent::ProbeDone { took_ms: 40 },
            DiagEvent::BackfillDone { samples: 1200 },
            DiagEvent::WindowReady { took_ms: 310 },
            DiagEvent::NotifyFailed,
            DiagEvent::AutostartFailed,
            DiagEvent::Started,
            DiagEvent::Stopping,
        ]
    }

    #[test]
    fn lines_are_exactly_these() {
        let lines: Vec<String> = every_event().into_iter().map(line).collect();
        assert_eq!(
            lines,
            [
                "flashboard: SourceOk source=CodexAppServer took_ms=2750",
                "flashboard: SourceFailed source=DeepSeekBalance reason=Unauthorized",
                "flashboard: StoreFailed error=Corrupt",
                "flashboard: LogRotated",
                "flashboard: WatchLost",
                "flashboard: WatchArmed",
                "flashboard: ClockChanged",
                "flashboard: ProbeDone took_ms=40",
                "flashboard: BackfillDone samples=1200",
                "flashboard: WindowReady took_ms=310",
                "flashboard: NotifyFailed",
                "flashboard: AutostartFailed",
                "flashboard: Started",
                "flashboard: Stopping",
            ]
        );
    }

    #[test]
    fn every_line_is_one_line_of_names_and_numbers() {
        for event in every_event() {
            let text = line(event);
            assert!(!text.contains('\n'));
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b" :=_".contains(&b)),
                "unexpected character in {text:?}"
            );
            assert!(!text.contains('/') && !text.contains('@'));
        }
    }

    #[test]
    fn capture_collects_in_order_and_ends_with_the_guard() {
        let guard = capture();
        assert!(guard.events().is_empty());
        emit(DiagEvent::Started);
        emit(DiagEvent::ProbeDone { took_ms: 7 });
        assert_eq!(
            guard.events(),
            [DiagEvent::Started, DiagEvent::ProbeDone { took_ms: 7 }]
        );
        drop(guard);
        // Not captured any more: goes to stderr, and a new capture starts empty.
        emit(DiagEvent::Stopping);
        let again = capture();
        assert!(again.events().is_empty());
    }

    #[test]
    fn captures_take_turns() {
        let handles: Vec<std::thread::JoinHandle<()>> = (0..4_u32)
            .map(|n| {
                std::thread::spawn(move || {
                    let guard = capture();
                    for _ in 0..50 {
                        emit(DiagEvent::BackfillDone { samples: n });
                    }
                    assert_eq!(
                        guard.events(),
                        vec![DiagEvent::BackfillDone { samples: n }; 50]
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
