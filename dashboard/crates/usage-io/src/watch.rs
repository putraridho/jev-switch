//! The file watch that signals the gateway-log source (DESIGN 7.2). The types are frozen;
//! T03 writes the body.

use jev_usage_core::port::SourceId;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

/// Dropping it stops the watch and its re-arm timer.
#[derive(Debug)]
pub struct WatchGuard {
    _private: (),
}

/// The watch's timings.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct WatchOptions {
    /// The throttle: the first event arms this timer and the signal is sent when it fires.
    pub coalesce: Duration,
    /// The first wait before re-arming a lost watch.
    pub rearm_first: Duration,
    /// The longest wait between re-arm attempts.
    pub rearm_max: Duration,
}

impl Default for WatchOptions {
    /// 1.5 s, 1 min, 15 min.
    fn default() -> Self {
        Self {
            coalesce: Duration::from_millis(1500),
            rearm_first: Duration::from_mins(1),
            rearm_max: Duration::from_mins(15),
        }
    }
}

/// inotify (its descriptor in tokio's `AsyncFd` on `rt`'s reactor, no thread and no stream
/// adapter): MODIFY on the file, and CREATE | MOVED_TO | MOVED_FROM | DELETE on the parent
/// directory filtered to the file name. Throttled: the first event arms `coalesce`, the
/// signal is sent when it fires. Never fails: when the directory is missing or the watch is
/// lost it re-arms itself at 1, 2, 4 … 15 min and signals once when armed. A queue overflow
/// is a signal.
pub fn watch_file(
    path: &Path,
    id: SourceId,
    tx: mpsc::Sender<SourceId>,
    opts: WatchOptions,
    rt: &tokio::runtime::Handle,
) -> WatchGuard {
    let _ = (path, id, tx, opts, rt);
    WatchGuard { _private: () }
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
    fn default_options_are_the_design_values() {
        let opts = WatchOptions::default();
        assert_eq!(opts.coalesce, Duration::from_millis(1500));
        assert_eq!(opts.rearm_first.as_secs(), 60);
        assert_eq!(opts.rearm_max.as_secs(), 900);
    }
}
