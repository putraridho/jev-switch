//! Small text helpers shared by the formatters: one-line cleaning, plurals, and the
//! way counts and durations read.

pub(crate) use crate::shared::text::one_line;

/// A token count the way people read it: `189`, `1.2K`, `45K`, `1.2M`.
pub fn human_count(n: u64) -> String {
    fn scaled(n: u64, unit: u64, suffix: &str) -> String {
        if n < unit * 10 {
            let tenths = (n * 10 + unit / 2) / unit;
            if tenths.is_multiple_of(10) {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            }
        } else {
            format!("{}{suffix}", (n + unit / 2) / unit)
        }
    }
    if n < 1000 {
        n.to_string()
    } else if (n + 500) / 1000 >= 1000 {
        scaled(n, 1_000_000, "M")
    } else if n < 1_000_000 {
        scaled(n, 1000, "K")
    } else {
        scaled(n, 1_000_000, "M")
    }
}

/// A duration the way people read it: `850ms`, `14s`, `2m05s`, `1h05m`.
pub fn human_duration(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = (ms + 500) / 1000;
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60),
    }
}

/// A time of day the way a clock on the wall says it: `6:27 PM`, `12:05 AM`.
pub fn clock_12h(hour: u32, minute: u32) -> String {
    let (h, half) = match hour % 24 {
        0 => (12, "AM"),
        h @ 1..=11 => (h, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!("{h}:{minute:02} {half}")
}

/// The local time now, as [`clock_12h`] says it. `None` if the system cannot tell.
pub fn local_clock() -> Option<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let t = libc::time_t::try_from(now).ok()?;
    // SAFETY: `localtime_r` writes only into the `tm` we own and hand it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let filled = unsafe { libc::localtime_r(&t, &mut tm) };
    if filled.is_null() {
        return None;
    }
    Some(clock_12h(u32::try_from(tm.tm_hour).ok()?, u32::try_from(tm.tm_min).ok()?))
}

pub(super) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub(super) fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_humanised() {
        for (n, s) in [
            (0, "0"),
            (189, "189"),
            (999, "999"),
            (1000, "1K"),
            (1234, "1.2K"),
            (9_960, "10K"),
            (45_000, "45K"),
            (45_678, "46K"),
            (999_400, "999K"),
            (999_999, "1M"),
            (1_000_000, "1M"),
            (1_200_000, "1.2M"),
            (45_300_000, "45M"),
        ] {
            assert_eq!(human_count(n), s, "{n}");
        }
    }

    #[test]
    fn a_clock_reads_like_a_wall_clock() {
        for (h, m, s) in [(0, 5, "12:05 AM"), (9, 0, "9:00 AM"), (12, 30, "12:30 PM"), (18, 27, "6:27 PM"), (23, 59, "11:59 PM")] {
            assert_eq!(clock_12h(h, m), s);
        }
        assert!(local_clock().is_some_and(|c| c.ends_with('M')));
    }

    #[test]
    fn durations_are_humanised() {
        for (ms, s) in [
            (0, "0ms"),
            (850, "850ms"),
            (999, "999ms"),
            (1000, "1s"),
            (14_000, "14s"),
            (14_400, "14s"),
            (59_400, "59s"),
            (59_600, "1m00s"),
            (125_000, "2m05s"),
            (3_599_000, "59m59s"),
            (3_900_000, "1h05m"),
        ] {
            assert_eq!(human_duration(ms), s, "{ms}");
        }
    }
}
