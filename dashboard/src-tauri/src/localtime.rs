//! Local time for tray and notification text (DESIGN 7.8). T15 writes the body.

use jev_usage_core::text::{LocalStamp, LocalTime};
use jev_usage_core::time::UnixSeconds;

/// The system zone, re-read on each call so a zone change is picked up.
#[derive(Copy, Clone, Debug, Default)]
pub struct SystemLocalTime;

impl LocalTime for SystemLocalTime {
    fn local(&self, at: UnixSeconds) -> LocalStamp {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let secs = i64::try_from(at.0).unwrap_or(0);
        match Local.timestamp_opt(secs, 0).single() {
            Some(dt) => LocalStamp {
                year: dt.year(),
                month: u8::try_from(dt.month()).unwrap_or(1),
                day: u8::try_from(dt.day()).unwrap_or(1),
                weekday: u8::try_from(dt.weekday().num_days_from_monday()).unwrap_or(0),
                hour: u8::try_from(dt.hour()).unwrap_or(0),
                minute: u8::try_from(dt.minute()).unwrap_or(0),
            },
            None => LocalStamp {
                year: 1970,
                month: 1,
                day: 1,
                weekday: 3,
                hour: 0,
                minute: 0,
            },
        }
    }
}
