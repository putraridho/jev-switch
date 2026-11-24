//! The only place tray and notification strings are built (DESIGN 9.9, 9.11, 9.12), from
//! typed inputs only. Not a type file. The types are frozen; T02 writes the function bodies.

use crate::alerts::Alert;
use crate::ipc::{HourCycle, Snapshot};
use crate::meter::WindowKind;
use crate::provider::ProviderKey;
use crate::quantity::Percent;
use crate::time::UnixSeconds;

/// A wall-clock moment in the user's zone, as plain numbers.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct LocalStamp {
    /// The calendar year.
    pub year: i32,
    /// The month, 1..=12.
    pub month: u8,
    /// The day of the month, 1..=31.
    pub day: u8,
    /// The day of the week, 0 (Monday) ..= 6 (Sunday).
    pub weekday: u8,
    /// The hour, 0..=23.
    pub hour: u8,
    /// The minute, 0..=59.
    pub minute: u8,
}

/// The conversion to local time, passed in so core never asks for the zone (DESIGN 7.8).
pub trait LocalTime: Send + Sync {
    /// `at` in the user's zone.
    fn local(&self, at: UnixSeconds) -> LocalStamp;
}

/// One row of the tray menu; selecting it opens its provider (DESIGN 9.11).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct TrayRow {
    /// The provider the row opens.
    pub provider: ProviderKey,
    /// The row's text.
    pub text: String,
}

/// A percentage for display: whole, rounded down, "<1%" below one (DESIGN 9.9).
pub fn percent(p: Percent) -> String {
    let _ = p;
    String::new()
}

/// "5-hour", "7-day", "30-day", "N-hour"/"N-day", or the raw label.
pub fn window_name(k: &WindowKind) -> String {
    let _ = k;
    String::new()
}

/// "today 14:00", "tomorrow 02:00", "Thu 02:00" or "16 Dec 08:37".
pub fn reset_absolute(
    at: UnixSeconds,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> String {
    let _ = (at, now, local, hour);
    String::new()
}

/// "3d 4h", "2h 14m", "38m" or "under a minute"; `None` when `at` has passed.
pub fn countdown(at: UnixSeconds, now: UnixSeconds) -> Option<String> {
    let _ = (at, now);
    None
}

/// The tray's reset time: "14:00", "Thu 02:00" or "16 Dec"; never "today" or "tomorrow".
pub fn tray_reset(
    at: UnixSeconds,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> String {
    let _ = (at, now, local, hour);
    String::new()
}

/// "16 Dec 12:31": always date and time.
pub fn stale_stamp(at: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String {
    let _ = (at, local, hour);
    String::new()
}

/// The tray menu's rows: reading, stale, elapsed and no-reading rows (DESIGN 9.11).
pub fn tray_rows(
    snapshot: &Snapshot,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> Vec<TrayRow> {
    let _ = (snapshot, now, local, hour);
    Vec::new()
}

/// A notification's title and body (absolute time only, DESIGN 9.12).
pub fn notification(
    alert: &Alert,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> (String, String) {
    let _ = (alert, now, local, hour);
    (String::new(), String::new())
}
