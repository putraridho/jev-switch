use super::super::duration::parse_duration_ms;
use super::super::{cooldown_ms_with, DAY_MS, HOUR_MS, MAX_MS, MINUTE_MS, SECOND_MS};
use super::{never, NOW};

#[test]
fn parses_durations_in_every_form_gemini_and_others_use() {
    let cases = [
        ("3h20m5s", 3 * HOUR_MS + 20 * MINUTE_MS + 5 * SECOND_MS),
        ("20m", 20 * MINUTE_MS),
        ("3h 20m", 3 * HOUR_MS + 20 * MINUTE_MS),
        ("45s", MINUTE_MS), // under the floor
        ("90 minutes", 90 * MINUTE_MS),
        ("2 hours", 2 * HOUR_MS),
        ("1 hour", HOUR_MS),
        ("in 5 hrs", 5 * HOUR_MS),
        ("1d", DAY_MS),
        ("2H30M", 2 * HOUR_MS + 30 * MINUTE_MS),
        ("30 days", MAX_MS),
    ];
    for (text, want) in cases {
        assert_eq!(cooldown_ms_with(Some(text), NOW, &never), want, "{text}");
    }
}

#[test]
fn near_misses_are_not_durations() {
    for text in ["3x", "h", "3h20", "3h and 20m", "m5", "-5m", "5mm"] {
        assert_eq!(parse_duration_ms(text), None, "{text}");
    }
    assert_eq!(parse_duration_ms("99999999999999999999999d"), None); // overflows u64
    assert_eq!(parse_duration_ms(&format!("{}d", u64::MAX)), Some(u64::MAX)); // saturates, no panic
}
