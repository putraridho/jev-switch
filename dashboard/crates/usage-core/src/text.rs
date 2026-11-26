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

fn provider_name(p: ProviderKey) -> &'static str {
    match p {
        ProviderKey::Claude => "Claude",
        ProviderKey::Codex => "Codex",
        ProviderKey::DeepSeek => "DeepSeek",
        ProviderKey::Gemini => "Gemini",
        ProviderKey::Antigravity => "Antigravity",
        ProviderKey::Grok => "Grok",
    }
}

fn day_number(s: LocalStamp) -> i64 {
    let y = i64::from(s.year).saturating_sub(i64::from(s.month <= 2));
    let m = i64::from(s.month);
    let adjusted = if m > 2 {
        m.saturating_sub(3)
    } else {
        m.saturating_add(9)
    };
    y.saturating_mul(365)
        .saturating_add(y.div_euclid(4))
        .saturating_sub(y.div_euclid(100))
        .saturating_add(y.div_euclid(400))
        .saturating_add(adjusted.saturating_mul(153).saturating_add(2) / 5)
        .saturating_add(i64::from(s.day))
}
fn clock(s: LocalStamp, hour: HourCycle) -> String {
    if hour == HourCycle::H12 {
        let h = s.hour % 12;
        format!(
            "{}:{:02} {}",
            if h == 0 { 12 } else { h },
            s.minute,
            if s.hour < 12 { "AM" } else { "PM" }
        )
    } else {
        format!("{:02}:{:02}", s.hour, s.minute)
    }
}
fn month(m: u8) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
}
fn weekday(d: u8) -> &'static str {
    match d {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        6 => "Sun",
        _ => "",
    }
}
fn date(s: LocalStamp, include_year: bool) -> String {
    if include_year {
        format!("{} {} {}", s.day, month(s.month), s.year)
    } else {
        format!("{} {}", s.day, month(s.month))
    }
}

/// A percentage for display: whole, rounded down, "<1%" below one (DESIGN 9.9).
pub fn percent(p: Percent) -> String {
    let bp = p.basis_points();
    if bp > 0 && bp < 100 {
        "<1%".to_owned()
    } else {
        format!("{}%", bp / 100)
    }
}
/// "5-hour", "7-day", "30-day", "N-hour"/"N-day", a known Antigravity window's words, or the raw label.
pub fn window_name(k: &WindowKind) -> String {
    match k {
        WindowKind::FiveHour => "5-hour".to_owned(),
        WindowKind::SevenDay => "7-day".to_owned(),
        WindowKind::Monthly => "30-day".to_owned(),
        WindowKind::Rolling { minutes } if minutes % 1440 == 0 => format!("{}-day", minutes / 1440),
        WindowKind::Rolling { minutes } => format!("{}-hour", minutes / 60),
        WindowKind::Named { label } => match label.as_str() {
            "gemini_5h" => "Gemini 5-hour".to_owned(),
            "gemini_7d" => "Gemini 7-day".to_owned(),
            "claude_gpt_5h" => "Claude/GPT 5-hour".to_owned(),
            "claude_gpt_7d" => "Claude/GPT 7-day".to_owned(),
            other => other.to_owned(),
        },
    }
}
/// "today 14:00", "tomorrow 02:00", "Thu 02:00" or "16 Dec 08:37".
pub fn reset_absolute(
    at: UnixSeconds,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> String {
    let target = local.local(at);
    let current = local.local(now);
    let days = day_number(target).saturating_sub(day_number(current));
    let prefix = match days {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        2..=6 => weekday(target.weekday).to_owned(),
        _ => date(target, target.year != current.year),
    };
    format!("{} {}", prefix, clock(target, hour))
}
/// "3d 4h", "2h 14m", "38m" or "under a minute"; `None` when `at` has passed.
pub fn countdown(at: UnixSeconds, now: UnixSeconds) -> Option<String> {
    let seconds = at.0.checked_sub(now.0)?;
    if seconds == 0 {
        None
    } else if seconds >= 172_800 {
        Some(format!(
            "{}d {}h",
            seconds / 86_400,
            (seconds % 86_400) / 3_600
        ))
    } else if seconds >= 3_600 {
        Some(format!(
            "{}h {:02}m",
            seconds / 3_600,
            (seconds % 3_600) / 60
        ))
    } else if seconds >= 60 {
        Some(format!("{}m", seconds / 60))
    } else {
        Some("under a minute".to_owned())
    }
}
/// The tray's reset time: "14:00", "Thu 02:00" or "16 Dec"; never "today" or "tomorrow".
pub fn tray_reset(
    at: UnixSeconds,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> String {
    let target = local.local(at);
    let current = local.local(now);
    let delta = at.0.saturating_sub(now.0);
    if at.0 >= now.0 && delta < 86_400 {
        clock(target, hour)
    } else if at.0 >= now.0 && delta < 6 * 86_400 {
        format!("{} {}", weekday(target.weekday), clock(target, hour))
    } else {
        date(target, target.year != current.year)
    }
}
/// "16 Dec 12:31": always date and time.
pub fn stale_stamp(at: UnixSeconds, local: &dyn LocalTime, hour: HourCycle) -> String {
    let s = local.local(at);
    format!("{} {}", date(s, false), clock(s, hour))
}
/// The tray menu's rows: reading, stale, elapsed and no-reading rows (DESIGN 9.11).
pub fn tray_rows(
    snapshot: &Snapshot,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> Vec<TrayRow> {
    use crate::ipc::{MeterView, Phase, UsageView};
    let mut rows = Vec::new();
    for p in &snapshot.providers {
        if !p.enabled {
            continue;
        }
        let name = provider_name(p.provider);
        let (reading, stale) = match &p.usage {
            UsageView::Known { reading } => (reading, false),
            UsageView::Stale { reading, .. } => (reading, true),
            other => {
                let phrase = match other {
                    UsageView::Pending => "checking",
                    UsageView::Unavailable { reason } => match reason {
                        crate::usage::Unavailable::NotInstalled => "not installed",
                        crate::usage::Unavailable::NotLoggedIn => "not signed in",
                        crate::usage::Unavailable::Offline => "offline",
                        crate::usage::Unavailable::Timeout => "timed out",
                        crate::usage::Unavailable::RateLimited => "rate limited",
                        crate::usage::Unavailable::Unauthorized => "sign-in required",
                        crate::usage::Unavailable::ServerError => "server error",
                        crate::usage::Unavailable::Protocol => "unexpected response",
                        crate::usage::Unavailable::Io => "could not read data",
                        crate::usage::Unavailable::Internal => "something went wrong",
                    },
                    UsageView::NotConfigured { setup } => match setup {
                        crate::usage::Setup::UseGateway => "use gateway to start",
                        crate::usage::Setup::AddDeepSeekKey => "add DeepSeek key",
                    },
                    _ => continue,
                };
                rows.push(TrayRow {
                    provider: p.provider,
                    text: format!("{name} · {phrase}"),
                });
                continue;
            }
        };
        for meter in &reading.meters {
            if let MeterView::Window {
                window,
                used_bp,
                resets_at,
                phase,
                ..
            } = meter
            {
                let prefix = format!("{name} · {}", window_name(window));
                let text = if *phase == Phase::Elapsed {
                    format!("{prefix} · no data since reset")
                } else {
                    let value = percent(Percent::from_basis_points(u32::from(*used_bp)));
                    if stale {
                        format!(
                            "{prefix} {value} · as of {}",
                            stale_stamp(UnixSeconds(reading.observed_at / 1000), local, hour)
                        )
                    } else if let Some(at) = resets_at {
                        format!(
                            "{prefix} {value} · resets {}",
                            tray_reset(UnixSeconds(*at), now, local, hour)
                        )
                    } else {
                        format!("{prefix} {value}")
                    }
                };
                rows.push(TrayRow {
                    provider: p.provider,
                    text,
                });
            }
        }
    }
    rows
}
/// A notification's title and body (absolute time only, DESIGN 9.12).
pub fn notification(
    alert: &Alert,
    now: UnixSeconds,
    local: &dyn LocalTime,
    hour: HourCycle,
) -> (String, String) {
    let title = format!(
        "{} {} at {}",
        provider_name(alert.provider),
        window_name(&alert.window),
        percent(alert.used)
    );
    let body = alert.resets_at.map_or_else(String::new, |at| {
        format!("Resets {}.", reset_absolute(at, now, local, hour))
    });
    (title, body)
}
