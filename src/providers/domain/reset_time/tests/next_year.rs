use std::cell::RefCell;

use super::super::wall_clock::{names_date_without_year, utc_year};
use super::super::{DAY_MS, HOUR_MS, MAX_MS, MIN_MS};
use super::{cooldown_in, fixed, NEW_YEARS_EVE};

/// A resolver that behaves like `date` without a year: text with no year is read in
/// 2026, text ending in a year is read in that year. Records every call.
fn like_date(seen: &RefCell<Vec<String>>) -> impl Fn(&str, Option<&str>) -> Option<u64> + '_ {
    const JAN_2_5PM_2026: u64 = 1_767_373_200;
    const YEAR_SECS: u64 = 365 * 86_400;
    move |text, _| {
        seen.borrow_mut().push(text.to_string());
        let years_on = text.rsplit(' ').next().and_then(|w| w.parse::<u64>().ok()).map_or(0, |y| y - 2026);
        Some(JAN_2_5PM_2026 + years_on * YEAR_SECS)
    }
}

#[test]
fn a_date_without_a_year_that_has_passed_means_next_year() {
    // Said on Dec 31: `date` reads "Jan 2 5pm" as Jan 2 of THIS year, 364 days ago.
    // Before the fix that clamped to one minute and the provider was retried every
    // minute for two days.
    let seen = RefCell::new(vec![]);
    let resolve = like_date(&seen);
    let want = 2 * DAY_MS + 5 * HOUR_MS; // Jan 2 17:00 minus Dec 31 12:00
    assert_eq!(cooldown_in(Some("Jan 2 5pm (Asia/Jakarta)"), NEW_YEARS_EVE, &resolve), want);
    assert_eq!(*seen.borrow(), ["Jan 2 5pm", "Jan 2 5pm 2027"]);
    assert_eq!(cooldown_in(Some("January 2, 5pm (UTC)"), NEW_YEARS_EVE, &resolve), want);
    assert_eq!(seen.borrow().last().unwrap(), "January 2 5pm 2027");
}

#[test]
fn next_year_is_taken_only_when_in_the_future_and_within_the_cap() {
    let seen = RefCell::new(vec![]);
    // Still in the past next year too (a resolver that ignores the year): stale, one minute.
    let stale = fixed(NEW_YEARS_EVE / 1000 - 5, &seen);
    assert_eq!(cooldown_in(Some("Jan 2 5pm (UTC)"), NEW_YEARS_EVE, &stale), MIN_MS);
    // Next year but months away: not a reset anyone meant, not taken.
    let far = |text: &str, _: Option<&str>| Some(if text.ends_with("2027") { NEW_YEARS_EVE / 1000 + 62 * 86_400 } else { 1_000 });
    assert_eq!(cooldown_in(Some("Mar 3 5pm (UTC)"), NEW_YEARS_EVE, &far), MIN_MS);
    // Just inside the cap it is taken.
    let edge = |text: &str, _: Option<&str>| Some(if text.ends_with("2027") { NEW_YEARS_EVE / 1000 + 8 * 86_400 } else { 1_000 });
    assert_eq!(cooldown_in(Some("Jan 8 5pm (UTC)"), NEW_YEARS_EVE, &edge), MAX_MS);
}

#[test]
fn a_date_that_names_its_year_or_is_not_a_date_is_not_asked_twice() {
    let seen = RefCell::new(vec![]);
    let past = fixed(NEW_YEARS_EVE / 1000 - 3600, &seen);
    for text in ["Jan 2 2026 5pm (UTC)", "2026-01-02T17:00:00Z", "tomorrow 9am"] {
        assert_eq!(cooldown_in(Some(text), NEW_YEARS_EVE, &past), MIN_MS, "{text}");
    }
    assert_eq!(seen.borrow().len(), 3, "one resolve each");
    // A bare time of day keeps its plus-one-day rule, never a year.
    assert_eq!(cooldown_in(Some("5pm (UTC)"), NEW_YEARS_EVE, &past), 23 * HOUR_MS);
    assert_eq!(seen.borrow().len(), 4);
}

#[test]
fn year_of_an_instant_and_date_without_year_detection() {
    assert_eq!(utc_year(0), 1970);
    assert_eq!(utc_year(NEW_YEARS_EVE), 2026);
    assert_eq!(utc_year(NEW_YEARS_EVE + 12 * HOUR_MS), 2027);
    assert_eq!(utc_year(NEW_YEARS_EVE - 365 * DAY_MS), 2025);
    assert_eq!(utc_year(951_782_400_000), 2000); // 2000-02-29
    for yes in ["Jan 2 5pm", "January 2 17:00", "Nov 20 5:30pm", "Sept. 3 9am", "may 4 1pm"] {
        assert!(names_date_without_year(yes), "{yes}");
    }
    for no in ["Jan 2 2027 5pm", "5pm", "17:00", "2026-11-20T17:00:00Z", "tomorrow 9am", "mayor 5pm", "Monday 5pm"] {
        assert!(!names_date_without_year(no), "{no}");
    }
}
