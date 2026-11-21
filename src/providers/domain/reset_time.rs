//! Remembering which providers hit their usage limit, so the router skips them
//! until they reset instead of paying a failed turn to rediscover it.
//!
//! Time is always passed in (`now_ms`), wall-clock text is resolved through an
//! injected [`Resolver`], zone names are checked through an injected [`ZoneExists`],
//! and the file path is always a parameter, so every rule here is testable without a
//! clock, `date`, the zone database, or the real state directory.

pub mod cooldowns;
mod duration;
mod resolver;
#[cfg(test)]
mod tests;
mod wall_clock;

use resolver::{system_date, system_zone_exists};
use wall_clock::wall_clock_ms;

const SECOND_MS: u64 = 1000;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;
/// Used when the CLI gave no reset time we can read. An hour is long enough not to
/// hammer a limited provider and short enough to recover the same day.
const DEFAULT_MS: u64 = HOUR_MS;
/// Never trust a parsed reset to be shorter than this (clock skew, "resets now").
const MIN_MS: u64 = MINUTE_MS;
/// Never wait longer than the longest plan window (weekly limits), so one bad
/// timestamp cannot disable a provider for months.
const MAX_MS: u64 = 8 * DAY_MS;
/// Epoch values at or above this are milliseconds; below it, seconds. 1e11 s is the
/// year 5138 and 1e11 ms is 1973, so real timestamps never straddle the line.
const MS_THRESHOLD: u64 = 100_000_000_000;
/// Reset text longer than this is not a reset time; it is never handed to `date`.
const MAX_TEXT_LEN: usize = 64;

/// Turns wall-clock text (and an optional IANA zone) into epoch seconds, or `None`.
/// Injected so tests never run `date`; production passes [`system_date`].
pub type Resolver<'a> = &'a dyn Fn(&str, Option<&str>) -> Option<u64>;

/// Whether a zone name is a real IANA zone. Injected so tests need no zone database;
/// production passes [`system_zone_exists`].
pub type ZoneExists<'a> = &'a dyn Fn(&str) -> bool;


/// Milliseconds a provider should be skipped, given the CLI's reset text, resolving
/// wall-clock forms with the system `date`. See [`cooldown_ms_in`].
pub fn cooldown_ms(resets_at: Option<&str>, now_ms: u64) -> u64 {
    cooldown_ms_with(resets_at, now_ms, &system_date)
}

/// Milliseconds a provider should be skipped, given the CLI's reset text.
///
/// Understood forms: epoch seconds or milliseconds as digits; a duration
/// ("3h20m5s", "20m", "90 minutes", "2 hours"); a time of day or date and time
/// ("5pm (Asia/Jakarta)", "Nov 20, 5pm (Asia/Jakarta)") and ISO-8601, both resolved
/// by `resolve`. Zones are checked against the system zone database. See
/// [`cooldown_ms_in`] for the rules.
pub fn cooldown_ms_with(resets_at: Option<&str>, now_ms: u64, resolve: Resolver) -> u64 {
    cooldown_ms_in(resets_at, now_ms, resolve, &system_zone_exists)
}

/// [`cooldown_ms_with`] with an injected zone check: a zone is accepted only if it is
/// exactly `UTC`/`GMT` or `zone_exists` says it is a real IANA zone. An abbreviation
/// such as `PST` is ambiguous (and `date` would silently read it as UTC), so it is as
/// unreadable as any other unknown zone.
///
/// A bare time of day that is not in the future means tomorrow. A date with no year
/// that is not in the future (said on Dec 31: "Jan 2 5pm") means next year, if that is
/// within the 8-day cap. Anything unreadable (or nothing) means [`DEFAULT_MS`]; any
/// result is clamped to [1 minute, 8 days].
pub fn cooldown_ms_in(resets_at: Option<&str>, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> u64 {
    let reset_ms = resets_at
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| reset_instant_ms(s, now_ms, resolve, zone_exists));
    let wait = reset_ms.map_or(DEFAULT_MS, |r| r.saturating_sub(now_ms));
    wait.clamp(MIN_MS, MAX_MS)
}

/// The reset instant (epoch ms) the text names, by form: epoch, duration, wall clock.
fn reset_instant_ms(text: &str, now_ms: u64, resolve: Resolver, zone_exists: ZoneExists) -> Option<u64> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        let n: u64 = text.parse().ok()?;
        return Some(if n >= MS_THRESHOLD { n } else { n.saturating_mul(1000) });
    }
    if let Some(d) = duration::parse_duration_ms(text) {
        return Some(now_ms.saturating_add(d));
    }
    wall_clock_ms(text, now_ms, resolve, zone_exists)
}
