use std::cell::RefCell;

use super::super::wall_clock::is_bare_time_of_day;
use super::super::{cooldown_ms_with, DAY_MS, HOUR_MS, MAX_MS, MIN_MS};
use super::{cooldown_in, fixed, NOW};

#[test]
fn time_of_day_with_a_zone_goes_to_the_resolver() {
    let seen = RefCell::new(vec![]);
    let two_hours = fixed(NOW / 1000 + 7200, &seen);
    assert_eq!(cooldown_in(Some("5pm (Asia/Jakarta)"), NOW, &two_hours), 2 * HOUR_MS);
    assert_eq!(seen.borrow()[0], ("5pm".to_string(), Some("Asia/Jakarta".to_string())));
    assert_eq!(cooldown_ms_with(Some("5pm"), NOW, &two_hours), 2 * HOUR_MS);
    assert_eq!(seen.borrow()[1], ("5pm".to_string(), None));
}

#[test]
fn a_bare_time_of_day_already_past_means_tomorrow() {
    let seen = RefCell::new(vec![]);
    let an_hour_ago = fixed(NOW / 1000 - 3600, &seen);
    for text in ["5pm (Asia/Jakarta)", "5:30 pm", "17:00 (UTC)"] {
        assert_eq!(cooldown_in(Some(text), NOW, &an_hour_ago), 23 * HOUR_MS, "{text}");
    }
    // Exactly now is not in the future either.
    assert_eq!(cooldown_ms_with(Some("5pm"), NOW, &fixed(NOW / 1000, &seen)), DAY_MS);
}

#[test]
fn a_date_with_a_time_is_not_pushed_a_day() {
    let seen = RefCell::new(vec![]);
    let in_three_days = fixed(NOW / 1000 + 3 * 86_400, &seen);
    assert_eq!(cooldown_in(Some("Nov 20, 5pm (Asia/Jakarta)"), NOW, &in_three_days), 3 * DAY_MS);
    assert_eq!(seen.borrow()[0], ("Nov 20 5pm".to_string(), Some("Asia/Jakarta".to_string())));
    // In the past it is not "tomorrow": it clamps like any stale timestamp.
    let past = fixed(NOW / 1000 - 3600, &seen);
    assert_eq!(cooldown_in(Some("Nov 20, 5pm (Asia/Jakarta)"), NOW, &past), MIN_MS);
    // GNU date rejects the comma and "at"; both are dropped.
    cooldown_ms_with(Some("Nov 20 at 5pm"), NOW, &in_three_days);
    assert_eq!(seen.borrow().last().unwrap().0, "Nov 20 5pm");
}

#[test]
fn iso_timestamps_go_to_the_resolver_and_clamp() {
    let seen = RefCell::new(vec![]);
    let tomorrow = fixed(NOW / 1000 + 86_400, &seen);
    assert_eq!(cooldown_ms_with(Some("2026-11-20T17:00:00Z"), NOW, &tomorrow), DAY_MS);
    assert_eq!(seen.borrow()[0], ("2026-11-20T17:00:00Z".to_string(), None));
    cooldown_ms_with(Some("2026-11-20T17:00:00.250+07:00"), NOW, &tomorrow);
    assert_eq!(seen.borrow()[1].0, "2026-11-20T17:00:00.250+07:00");
    assert_eq!(cooldown_ms_with(Some("2030-01-01T00:00:00Z"), NOW, &fixed(NOW / 1000 + 400 * 86_400, &seen)), MAX_MS);
}

#[test]
fn bare_time_of_day_detection() {
    for yes in ["5pm", "5 PM", "11:30am", "17:00", "5:30:15", "5pm"] {
        assert!(is_bare_time_of_day(yes), "{yes}");
    }
    for no in ["Nov 20, 5pm", "2026-11-20T17:00:00Z", "tomorrow", "5", "123pm", "", "pm", "5:300"] {
        assert!(!is_bare_time_of_day(no), "{no}");
    }
}
