use super::super::{cooldown_ms_with, DEFAULT_MS, HOUR_MS, MAX_MS, MIN_MS};
use super::{never, unresolvable, NOW};

#[test]
fn default_is_one_hour() {
    // "tomorrow 9am" is a real date expression, so the resolver here knows nothing.
    for text in [None, Some(""), Some("tomorrow 9am"), Some("12abc")] {
        assert_eq!(cooldown_ms_with(text, NOW, &unresolvable), DEFAULT_MS, "{text:?}");
    }
}

#[test]
fn parses_epoch_seconds_and_milliseconds() {
    let secs = ((NOW + 2 * HOUR_MS) / 1000).to_string();
    let ms = (NOW + 2 * HOUR_MS).to_string();
    assert_eq!(cooldown_ms_with(Some(&secs), NOW, &never), 2 * HOUR_MS);
    assert_eq!(cooldown_ms_with(Some(&format!(" {ms} ")), NOW, &never), 2 * HOUR_MS);
}

#[test]
fn clamps_to_one_minute_and_eight_days() {
    let past = (NOW / 1000 - 5000).to_string();
    assert_eq!(cooldown_ms_with(Some(&past), NOW, &never), MIN_MS);
    let far = (NOW / 1000 + 90 * 86_400).to_string();
    assert_eq!(cooldown_ms_with(Some(&far), NOW, &never), MAX_MS);
    assert_eq!(cooldown_ms_with(Some("99999999999999999999999"), NOW, &never), DEFAULT_MS); // overflows u64
}
