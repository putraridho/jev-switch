//! Small text helpers shared by the formatters: one-line cleaning, plurals, and the
//! way counts and durations read.

pub use jev_shared::domain::text::one_line;
pub use jev_shared::domain::wording::{human_count, human_duration};

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
    fn a_clock_reads_like_a_wall_clock() {
        for (h, m, s) in [(0, 5, "12:05 AM"), (9, 0, "9:00 AM"), (12, 30, "12:30 PM"), (18, 27, "6:27 PM"), (23, 59, "11:59 PM")] {
            assert_eq!(clock_12h(h, m), s);
        }
        assert!(local_clock().is_some_and(|c| c.ends_with('M')));
    }
}
