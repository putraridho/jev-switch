//! Allow-list for `agy -p /usage --output-format json` (DESIGN 6.5). Only the status, the
//! bucket id, the remaining fraction and the reset time are read; names, descriptions and
//! every other key are skipped. Bucket ids map to four fixed labels, so no string from the
//! tool ever reaches the UI.

use crate::label::Label;
use crate::meter::{LimitStatus, Meter, Meters, WindowKind, WindowReading};
use crate::quantity::Percent;
use crate::time::UnixSeconds;
use crate::usage::Unavailable;
use serde::Deserialize;

/// How close a reset must be to "now plus the whole window" to count as a placeholder.
const BLANK_TOLERANCE_S: u64 = 120;

#[derive(Deserialize)]
struct Output<'a> {
    status: Option<&'a str>,
    #[serde(borrow)]
    command: Option<Command<'a>>,
}
#[derive(Deserialize)]
struct Command<'a> {
    #[serde(borrow)]
    data: Option<Data<'a>>,
}
#[derive(Deserialize)]
struct Data<'a> {
    #[serde(borrow)]
    groups: Vec<Group<'a>>,
}
#[derive(Deserialize)]
struct Group<'a> {
    #[serde(borrow)]
    buckets: Vec<Bucket<'a>>,
}
#[derive(Deserialize)]
struct Bucket<'a> {
    id: &'a str,
    remaining_fraction: f64,
    #[serde(borrow)]
    reset_time: Option<&'a str>,
}

/// The buckets this version knows, in the order they are shown: id, label, window length.
const KNOWN: [(&str, &str, u64); 4] = [
    ("gemini-5h", "gemini_5h", 5 * 3600),
    ("gemini-weekly", "gemini_7d", 7 * 86_400),
    ("3p-5h", "claude_gpt_5h", 5 * 3600),
    ("3p-weekly", "claude_gpt_7d", 7 * 86_400),
];

/// Reads one whole `/usage` answer observed at `now`.
///
/// `agy` answers `SUCCESS` with every window full when it is not signed in: each reset is
/// then either exactly one window ahead or absent (epoch 0). An answer in which no bucket
/// shows anything else is therefore reported as `NotLoggedIn`, never as real data.
///
/// # Errors
/// `Protocol` for an answer of another shape or without a known bucket, `ServerError` when
/// the status is not `SUCCESS`, `NotLoggedIn` for the blank answer.
pub fn parse_usage(output: &[u8], now: UnixSeconds) -> Result<Meters, Unavailable> {
    let parsed: Output<'_> = serde_json::from_slice(output).map_err(|_| Unavailable::Protocol)?;
    if parsed.status != Some("SUCCESS") {
        return Err(Unavailable::ServerError);
    }
    let groups = parsed
        .command
        .and_then(|c| c.data)
        .ok_or(Unavailable::Protocol)?
        .groups;
    let buckets: Vec<Bucket<'_>> = groups.into_iter().flat_map(|g| g.buckets).collect();
    let mut meters = Vec::new();
    let mut evidence = false;
    for (id, label, length) in KNOWN {
        let Some(bucket) = buckets.iter().find(|b| b.id == id) else {
            continue;
        };
        let remaining = bucket.remaining_fraction;
        if !(0.0..=1.0).contains(&remaining) {
            continue;
        }
        let (Some(used), Some(label)) =
            (Percent::from_fraction(1.0 - remaining), Label::parse(label))
        else {
            continue;
        };
        // Epoch 0 means "no reset": the tool says so for an untouched window.
        let reset = bucket.reset_time.and_then(parse_utc).filter(|at| at.0 > 0);
        let untouched = remaining >= 1.0
            && reset
                .is_none_or(|at| at.0.abs_diff(now.0.saturating_add(length)) <= BLANK_TOLERANCE_S);
        evidence |= !untouched;
        meters.push(Meter::Window(WindowReading {
            kind: WindowKind::Named { label },
            used,
            resets_at: reset,
            status: if remaining > 0.0 {
                LimitStatus::Allowed
            } else {
                LimitStatus::Rejected
            },
        }));
    }
    let meters = Meters::new(meters).ok_or(Unavailable::Protocol)?;
    if evidence {
        Ok(meters)
    } else {
        Err(Unavailable::NotLoggedIn)
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z` as seconds since the epoch; `None` for anything else.
fn parse_utc(text: &str) -> Option<UnixSeconds> {
    let body = text.strip_suffix('Z')?;
    let (date, time) = body.split_once('T')?;
    let time = time.split_once('.').map_or(time, |(whole, _)| whole);
    let mut date = date.split('-');
    let mut time = time.split(':');
    let field = |parts: &mut std::str::Split<'_, char>| -> Option<u64> {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 4 && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let (year, month, day) = (field(&mut date)?, field(&mut date)?, field(&mut date)?);
    let (hour, minute, second) = (field(&mut time)?, field(&mut time)?, field(&mut time)?);
    if date.next().is_some()
        || time.next().is_some()
        || year < 1970
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm), for years from 1970 on.
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = y / 400;
    let yoe = y % 400;
    let mp = (month.checked_add(9)?) % 12;
    let doy = (153_u64.checked_mul(mp)?.checked_add(2)? / 5).checked_add(day.checked_sub(1)?)?;
    let doe = yoe
        .checked_mul(365)?
        .checked_add(yoe / 4)?
        .checked_sub(yoe / 100)?
        .checked_add(doy)?;
    let days = era
        .checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)?;
    days.checked_mul(86_400)?
        .checked_add(hour.checked_mul(3600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)
        .map(UnixSeconds)
}
