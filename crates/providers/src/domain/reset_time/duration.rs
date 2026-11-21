//! Durations in reset text: "3h20m5s", "90 minutes", "in 1 day".

use super::{DAY_MS, HOUR_MS, MAX_TEXT_LEN, MINUTE_MS, SECOND_MS};

/// "3h20m5s", "3h 20m", "20m", "90 minutes", "2 hours", "in 1 day": a sum of
/// number+unit parts and nothing else.
pub(super) fn parse_duration_ms(text: &str) -> Option<u64> {
    if text.len() > MAX_TEXT_LEN {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.strip_prefix("in ").unwrap_or(&lower).trim_start();
    let mut total = 0u64;
    let mut parts = 0;
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let n: u64 = rest[..digits].parse().ok()?;
        rest = rest[digits..].trim_start();
        let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let unit_ms = match &rest[..letters] {
            "d" | "day" | "days" => DAY_MS,
            "h" | "hr" | "hrs" | "hour" | "hours" => HOUR_MS,
            "m" | "min" | "mins" | "minute" | "minutes" => MINUTE_MS,
            "s" | "sec" | "secs" | "second" | "seconds" => SECOND_MS,
            _ => return None,
        };
        total = total.saturating_add(n.saturating_mul(unit_ms));
        parts += 1;
        rest = rest[letters..].trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    }
    (parts > 0).then_some(total)
}
