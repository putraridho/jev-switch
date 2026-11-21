//! Wall-clock reset text: a time of day, a date and time, or ISO-8601, resolved
//! through an injected [`Resolver`].

use super::resolver::{is_safe_zone, zone_is_known};
use super::{Resolver, ZoneExists, DAY_MS, MAX_MS, MAX_TEXT_LEN};

/// Text `date -d` may be given: letters, digits, spaces and `, : / _ + - . ( )` only.
/// No quotes, `$`, `@` (epoch syntax), backticks, `;`, `&`, `|`, newlines or non-ASCII.
pub fn is_safe_text(text: &str) -> bool {
    text.len() <= MAX_TEXT_LEN && text.bytes().all(|b| b.is_ascii_alphanumeric() || b" ,:/_+-.()".contains(&b))
}

/// `"5pm (Asia/Jakarta)"` into `("5pm", Some("Asia/Jakarta"))`; without a trailing
/// parenthesis the whole text is the body.
pub fn split_zone(text: &str) -> Option<(&str, Option<&str>)> {
    let Some(inner) = text.strip_suffix(')') else { return (!text.contains(['(', ')'])).then_some((text, None)) };
    let open = inner.rfind('(')?;
    let (body, zone) = (inner[..open].trim_end(), inner[open + 1..].trim());
    (!body.is_empty() && !body.contains(['(', ')']) && is_safe_zone(zone)).then_some((body, Some(zone)))
}

/// "5pm", "5:30 pm", "17:00": a time of day with no date.
pub(super) fn is_bare_time_of_day(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let (clock, meridiem) = match lower.strip_suffix("am").or_else(|| lower.strip_suffix("pm")) {
        Some(c) => (c.trim_end(), true),
        None => (lower.as_str(), false),
    };
    let fields: Vec<&str> = clock.split(':').collect();
    let numeric = fields.iter().all(|f| (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()));
    numeric && fields.len() <= 3 && (meridiem || fields.len() > 1)
}

/// GNU `date` rejects "Nov 20, 5pm" and "Nov 20 at 5pm" but takes "Nov 20 5pm": drop
/// the commas and the word "at" from spaced text. Text without spaces (ISO-8601) is
/// left alone.
pub fn normalize_for_date(body: &str) -> String {
    if !body.contains(' ') {
        return body.to_string();
    }
    let spaced = body.replace(',', " ");
    spaced.split_whitespace().filter(|w| !w.eq_ignore_ascii_case("at")).collect::<Vec<_>>().join(" ")
}

const MONTHS: [&str; 24] = [
    "jan", "january", "feb", "february", "mar", "march", "apr", "april", "may", "jun", "june", "jul", "july", "aug",
    "august", "sep", "sept", "september", "oct", "october", "nov", "november", "dec", "december",
];

/// A date that names a month but no four-digit year: "Jan 2 5pm", "November 20".
pub(super) fn names_date_without_year(body: &str) -> bool {
    let words = || body.split_whitespace().map(|w| w.trim_end_matches('.'));
    let has_year = words().any(|w| w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit()));
    !has_year && words().any(|w| MONTHS.contains(&w.to_ascii_lowercase().as_str()))
}

/// The UTC calendar year of an instant (days since 1970 to a civil year).
pub(super) fn utc_year(ms: u64) -> u64 {
    let z = (ms / DAY_MS) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month = (5 * doy + 2) / 153; // 0 = March
    (yoe + era * 400 + i64::from(month >= 10)) as u64
}

/// A yearless date that came out in the past, asked again for next year. Taken only
/// if that is in the future and within [`MAX_MS`]: a stale date far ahead is more
/// likely a misread than a real reset.
fn next_year_instant(body: &str, zone: Option<&str>, now_ms: u64, resolve: Resolver) -> Option<u64> {
    let next = resolve(&format!("{body} {}", utc_year(now_ms) + 1), zone)?.saturating_mul(1000);
    (next > now_ms && next - now_ms <= MAX_MS).then_some(next)
}

/// A wall-clock reset (time of day, date and time, ISO-8601) via `resolve`.
pub(super) fn wall_clock_ms(text: &str, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> Option<u64> {
    if !is_safe_text(text) {
        return None;
    }
    let (body, zone) = split_zone(text)?;
    if zone.is_some_and(|z| !zone_is_known(z, zone_exists)) {
        return None;
    }
    let body = normalize_for_date(body);
    let ms = resolve(&body, zone)?.saturating_mul(1000);
    if ms > now_ms {
        return Some(ms);
    }
    if is_bare_time_of_day(&body) {
        return Some(ms.saturating_add(DAY_MS)); // "resets 5pm" said after 5pm means tomorrow
    }
    if names_date_without_year(&body) {
        return Some(next_year_instant(&body, zone, now_ms, resolve).unwrap_or(ms));
    }
    Some(ms)
}
