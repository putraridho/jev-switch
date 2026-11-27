//! The file watch that signals the gateway-log source (DESIGN 7.2). The types are frozen;
//! T03 writes the body.

use jev_usage_core::port::SourceId;
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// Dropping it stops the watch and its re-arm timer.
#[derive(Debug)]
pub struct WatchGuard {
    stop: Arc<AtomicBool>,
    handle: AbortHandle,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.abort();
    }
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

/// inotify: `MODIFY` on the file, and `CREATE | MOVED_TO | MOVED_FROM | DELETE` on the parent
/// directory filtered to the file name. The read is a blocking `read` on a blocking thread
/// (tokio's `AsyncFd` is unsound/deprecated in the pinned tokio, so no stream adapter is
/// used). Throttled: the first event arms `coalesce`, the signal is sent when it fires.
/// Never fails: when the directory is missing or the watch is lost it re-arms itself at
/// 1, 2, 4 … 15 min and signals once when armed. A queue overflow is a signal.
pub fn watch_file(
    path: &Path,
    id: SourceId,
    tx: mpsc::Sender<SourceId>,
    opts: WatchOptions,
    rt: &tokio::runtime::Handle,
) -> WatchGuard {
    let path = path.to_path_buf();
    let file_name = path.file_name().map(OsString::from);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_worker = Arc::clone(&stop);
    let handle = rt.spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            watch_blocking(&path, file_name.as_deref(), id, &tx, opts, &stop_worker);
        })
        .await;
    });
    WatchGuard {
        stop,
        handle: handle.abort_handle(),
    }
}

#[cfg(target_os = "linux")]
fn watch_blocking(
    path: &Path,
    file_name: Option<&std::ffi::OsStr>,
    id: SourceId,
    tx: &mpsc::Sender<SourceId>,
    opts: WatchOptions,
    stop: &AtomicBool,
) {
    use inotify::{EventMask, Inotify, WatchMask};

    let mut buffer = [0_u8; 4096];
    let mut rearm = opts.rearm_first;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(mut inotify) = Inotify::init() else {
            std::thread::sleep(rearm);
            rearm = next_rearm(rearm, opts);
            continue;
        };
        let file_wd = inotify.watches().add(path, WatchMask::MODIFY).ok();
        let _dir_wd = path.parent().and_then(|parent| {
            inotify
                .watches()
                .add(
                    parent,
                    WatchMask::CREATE
                        | WatchMask::MOVED_TO
                        | WatchMask::MOVED_FROM
                        | WatchMask::DELETE,
                )
                .ok()
        });

        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let Ok(events) = inotify.read_events_blocking(&mut buffer) else {
                break;
            };
            let mut hit = false;
            for event in events {
                if event.mask.contains(EventMask::Q_OVERFLOW) {
                    if tx.blocking_send(id).is_err() {
                        return;
                    }
                    continue;
                }
                let matches = match event.name {
                    Some(name) => Some(name) == file_name,
                    None => file_wd.as_ref().is_some_and(|wd| event.wd == *wd),
                };
                if matches {
                    hit = true;
                }
            }
            if hit {
                std::thread::sleep(opts.coalesce);
                if tx.blocking_send(id).is_err() {
                    return;
                }
            }
        }

        // Watch lost: re-arm with backoff.
        std::thread::sleep(rearm);
        rearm = next_rearm(rearm, opts);
    }
}

#[cfg(not(target_os = "linux"))]
fn watch_blocking(
    path: &Path,
    _file_name: Option<&std::ffi::OsStr>,
    id: SourceId,
    tx: &mpsc::Sender<SourceId>,
    opts: WatchOptions,
    stop: &AtomicBool,
) {
    let signature = || {
        std::fs::metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    // The state at arm time is the baseline: only a later change is a signal.
    let mut last_sig = signature();
    let poll_interval = opts.coalesce.max(Duration::from_millis(250));
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(poll_interval);
        let sig = signature();
        if sig != last_sig {
            last_sig = sig;
            if tx.blocking_send(id).is_err() {
                return;
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn next_rearm(current: Duration, opts: WatchOptions) -> Duration {
    current.saturating_mul(2).min(opts.rearm_max)
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

    #[test]
    fn rearm_doubles_to_the_maximum() {
        let opts = WatchOptions::default();
        assert_eq!(next_rearm(Duration::from_mins(1), opts).as_secs(), 120);
        assert_eq!(next_rearm(Duration::from_mins(8), opts).as_secs(), 900);
    }
}
