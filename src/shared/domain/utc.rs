//! UTC timestamps from epoch milliseconds, with no clock or locale involved.

/// `2026-11-17 10:04:05` UTC for an instant in ms since 1970, and the same with no
/// separators (`20261117-100405`) for file names.
pub fn utc_stamp(ms: u64) -> (String, String) {
    let secs = ms / 1000;
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC"), format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamps() {
        assert_eq!(utc_stamp(0), ("1970-01-01 00:00:00 UTC".to_string(), "19700101-000000".to_string()));
        // 2026-11-17 10:04:05 UTC
        assert_eq!(utc_stamp(1_794_909_845_000).0, "2026-11-17 10:04:05 UTC");
        assert_eq!(utc_stamp(951_782_400_000).0, "2000-02-29 00:00:00 UTC");
    }
}
