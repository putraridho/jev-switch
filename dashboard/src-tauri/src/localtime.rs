//! Local time for tray and notification text (DESIGN 7.8). T15 writes the body.

use jev_usage_core::text::{LocalStamp, LocalTime};
use jev_usage_core::time::UnixSeconds;

/// The system zone, re-read on each call so a zone change is picked up.
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemLocalTime;

impl LocalTime for SystemLocalTime {
    fn local(&self, at: UnixSeconds) -> LocalStamp {
        let _ = at;
        LocalStamp {
            year: 1970,
            month: 1,
            day: 1,
            weekday: 3,
            hour: 0,
            minute: 0,
        }
    }
}
