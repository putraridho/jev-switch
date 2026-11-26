//! Allow-list for Claude live rate-limit events.
use crate::meter::{LimitStatus, Meter, Meters, WindowKind, WindowReading};
use crate::quantity::Percent;
use crate::time::UnixSeconds;
use serde::Deserialize;

#[derive(Deserialize)]
struct Event<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    rate_limit_info: Info<'a>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Info<'a> {
    #[serde(borrow)]
    unified_windows: Windows<'a>,
}
#[derive(Deserialize)]
struct Windows<'a> {
    #[serde(borrow)]
    five_hour: Option<Window<'a>>,
    #[serde(borrow)]
    seven_day: Option<Window<'a>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window<'a> {
    utilization: f64,
    resets_at: Option<u64>,
    status: Option<&'a str>,
}
/// Parse a rate-limit event's windows.
pub fn parse_line(line: &[u8]) -> Option<Meters> {
    let event: Event<'_> = serde_json::from_slice(line).ok()?;
    if event.kind != "rate_limit_event" {
        return None;
    }
    let mut meters = Vec::new();
    for (window, kind) in [
        (
            event.rate_limit_info.unified_windows.five_hour,
            WindowKind::FiveHour,
        ),
        (
            event.rate_limit_info.unified_windows.seven_day,
            WindowKind::SevenDay,
        ),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(used) = Percent::from_fraction(window.utilization) else {
            continue;
        };
        let status = match window.status {
            Some("allowed") => LimitStatus::Allowed,
            Some("allowed_warning") => LimitStatus::Warning,
            Some("rejected") => LimitStatus::Rejected,
            _ => LimitStatus::Unknown,
        };
        meters.push(Meter::Window(WindowReading {
            kind,
            used,
            resets_at: window.resets_at.map(UnixSeconds),
            status,
        }));
    }
    Meters::new(meters)
}
