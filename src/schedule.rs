//! When a repeated run happens: `--every 10m` or `--every "*/15 9-17 * * 1-5"`.
//!
//! An interval is a number with a unit (`30s`, `10m`, `2h`, `1d`). A cron expression has the
//! usual five fields (minute hour day-of-month month day-of-week) with `*`, numbers, ranges
//! `a-b`, lists `a,b` and steps `*/n` or `a-b/n`; day-of-week is 0-7 with 0 and 7 Sunday.
//! Cron times are UTC (there is no time-zone data here). When both day fields are limited,
//! either one matching is enough, as in cron.
//!
//! Pure over epoch seconds, so every rule is a unit test; the loop in the binary only sleeps.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    Every(Duration),
    Cron(Cron),
}

/// `-p` run again and again: `--every <schedule> [--times <n>]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    pub schedule: Schedule,
    /// Stop after this many runs; `None`: until stopped with Ctrl-C.
    pub times: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    day_limited: bool,
    weekday_limited: bool,
}

/// A schedule that could not be read; the text says what to write instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleError(pub String);

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn bad<T>(message: impl Into<String>) -> Result<T, ScheduleError> {
    Err(ScheduleError(message.into()))
}

impl std::str::FromStr for Schedule {
    type Err = ScheduleError;

    fn from_str(text: &str) -> Result<Self, ScheduleError> {
        let text = text.trim();
        if text.split_whitespace().count() == 5 {
            return Cron::parse(text).map(Self::Cron);
        }
        interval(text).map(Self::Every)
    }
}

fn interval(text: &str) -> Result<Duration, ScheduleError> {
    let usage = || ScheduleError(format!("`{text}` is not a schedule: use an interval like 30s, 10m, 2h, 1d or a five-field cron expression"));
    let split = text.find(|c: char| !c.is_ascii_digit()).ok_or_else(usage)?;
    let (digits, unit) = text.split_at(split);
    let n: u64 = digits.parse().map_err(|_| usage())?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(usage()),
    };
    if n == 0 {
        return bad("an interval must be more than zero");
    }
    n.checked_mul(seconds).map(Duration::from_secs).ok_or_else(usage)
}

/// One cron field as a bit set over `lo..=hi`.
fn field(text: &str, name: &str, lo: u32, hi: u32) -> Result<u64, ScheduleError> {
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| ScheduleError(format!("{name}: bad step in `{part}`")))?),
            None => (part, 1),
        };
        let number = |s: &str| s.parse::<u32>().ok().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| ScheduleError(format!("{name}: `{s}` is not in {lo}-{hi}")));
        let (from, to) = match range {
            "*" => (lo, hi),
            r => match r.split_once('-') {
                Some((a, b)) => (number(a)?, number(b)?),
                None => {
                    let n = number(r)?;
                    (n, if part.contains('/') { hi } else { n })
                }
            },
        };
        if from > to {
            return bad(format!("{name}: `{part}` runs backwards"));
        }
        let mut v = from;
        while v <= to {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok(bits)
}

impl Cron {
    pub fn parse(text: &str) -> Result<Self, ScheduleError> {
        let f: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = f.as_slice() else {
            return bad("a cron expression has five fields: minute hour day-of-month month day-of-week");
        };
        let weekdays = field(weekday, "day-of-week", 0, 7)?;
        // 7 is Sunday too.
        let weekdays = (weekdays | (weekdays >> 7 & 1)) & 0x7f;
        Ok(Self {
            minutes: field(minute, "minute", 0, 59)?,
            hours: field(hour, "hour", 0, 23)?,
            days: field(day, "day-of-month", 1, 31)?,
            months: field(month, "month", 1, 12)?,
            weekdays,
            day_limited: *day != "*",
            weekday_limited: *weekday != "*",
        })
    }

    fn has(set: u64, n: u32) -> bool {
        set >> n & 1 == 1
    }

    fn day_matches(&self, month: u32, day: u32, weekday: u32) -> bool {
        let (d, w) = (Self::has(self.days, day), Self::has(self.weekdays, weekday));
        let day_ok = match (self.day_limited, self.weekday_limited) {
            (true, true) => d || w,
            (true, false) => d,
            (false, true) => w,
            (false, false) => true,
        };
        day_ok && Self::has(self.months, month)
    }

    /// The first matching minute strictly after `after` (epoch seconds, UTC), as epoch seconds.
    /// `None` when nothing matches within eight years (a date like 31 February).
    pub fn next_after(&self, after: u64) -> Option<u64> {
        let mut t = (after / 60 + 1) * 60;
        let limit = after + 8 * 366 * 86_400;
        while t <= limit {
            let (days, secs) = ((t / 86_400) as i64, t % 86_400);
            let (_, month, day) = civil_from_days(days);
            let weekday = (days + 4).rem_euclid(7) as u32;
            let (hour, minute) = ((secs / 3600) as u32, (secs % 3600 / 60) as u32);
            if !self.day_matches(month, day, weekday) {
                t = (days as u64 + 1) * 86_400;
            } else if !Self::has(self.hours, hour) {
                t = t - secs % 3600 + 3600;
            } else if !Self::has(self.minutes, minute) {
                t += 60;
            } else {
                return Some(t);
            }
        }
        None
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

impl Schedule {
    /// How long to wait from `now` (epoch seconds) until the next run.
    pub fn delay_from(&self, now: u64) -> Option<Duration> {
        match self {
            Self::Every(d) => Some(*d),
            Self::Cron(c) => c.next_after(now).map(|t| Duration::from_secs(t - now)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-18 (a Wednesday) 10:07:30 UTC.
    const NOW: u64 = 1_794_996_450;

    fn cron(text: &str) -> Cron {
        Cron::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn the_test_clock_is_what_the_comments_say() {
        assert_eq!(civil_from_days((NOW / 86_400) as i64), (2026, 11, 18));
        assert_eq!(((NOW / 86_400) as i64 + 4).rem_euclid(7), 3);
        assert_eq!(NOW % 86_400, 10 * 3600 + 7 * 60 + 30);
    }

    #[test]
    fn an_interval_too_large_to_hold_is_refused() {
        assert!("999999999999999999d".parse::<Schedule>().is_err());
        assert!("18446744073709551615s".parse::<Schedule>().is_ok());
    }

    #[test]
    fn intervals_have_units() {
        assert_eq!("90s".parse(), Ok(Schedule::Every(Duration::from_secs(90))));
        assert_eq!("10m".parse(), Ok(Schedule::Every(Duration::from_secs(600))));
        assert_eq!("2h".parse(), Ok(Schedule::Every(Duration::from_secs(7200))));
        assert_eq!("1d".parse(), Ok(Schedule::Every(Duration::from_secs(86_400))));
        for text in ["", "10", "m", "5x", "0m", "-5m", "1.5h"] {
            assert!(text.parse::<Schedule>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn cron_finds_the_next_matching_minute() {
        // Every 15 minutes: 10:15.
        assert_eq!(cron("*/15 * * * *").next_after(NOW), Some(NOW - 7 * 60 - 30 + 15 * 60));
        // Daily at 03:00: tomorrow.
        assert_eq!(cron("0 3 * * *").next_after(NOW), Some(NOW - 10 * 3600 - 7 * 60 - 30 + 86_400 + 3 * 3600));
        // The current minute does not count; the next one does.
        assert_eq!(cron("* * * * *").next_after(NOW), Some(NOW + 30));
        assert_eq!(cron("7 10 * * *").next_after(NOW), Some(NOW - 7 * 60 - 30 + 86_400 + 7 * 60));
    }

    #[test]
    fn weekdays_months_and_either_day_rule() {
        // Mon-Fri 09:00 from a Wednesday 10:07: Thursday.
        let start_of_day = NOW - NOW % 86_400;
        assert_eq!(cron("0 9 * * 1-5").next_after(NOW), Some(start_of_day + 86_400 + 9 * 3600));
        // Sunday as 0 and as 7 are the same: Sunday 2026-11-22.
        let sunday = start_of_day + 4 * 86_400;
        assert_eq!(cron("0 0 * * 0").next_after(NOW), Some(sunday));
        assert_eq!(cron("0 0 * * 7").next_after(NOW), Some(sunday));
        // Day 20 or a Friday (Nov 20, 2026 is both): the 20th at 00:00.
        assert_eq!(cron("0 0 20 * 5").next_after(NOW), Some(start_of_day + 2 * 86_400));
        // January only: next year, 2027-01-01 00:00 UTC.
        assert_eq!(cron("0 0 1 1 *").next_after(NOW), Some(1_798_761_600));
        // A date that never exists.
        assert_eq!(cron("0 0 31 2 *").next_after(NOW), None);
    }

    #[test]
    fn bad_cron_says_which_field() {
        for (text, needle) in [("61 * * * *", "minute"), ("* 24 * * *", "hour"), ("* * 0 * *", "day-of-month"), ("* * * 13 *", "month"), ("* * * * 8", "day-of-week"), ("*/0 * * * *", "step"), ("5-1 * * * *", "backwards")] {
            let e = Cron::parse(text).unwrap_err();
            assert!(e.0.contains(needle), "{text}: {e}");
        }
        assert!("* * *".parse::<Schedule>().is_err());
    }

    #[test]
    fn delay_is_the_interval_or_the_wait_for_cron() {
        assert_eq!("10m".parse::<Schedule>().unwrap().delay_from(NOW), Some(Duration::from_secs(600)));
        assert_eq!("*/15 * * * *".parse::<Schedule>().unwrap().delay_from(NOW), Some(Duration::from_secs(450)));
    }
}
