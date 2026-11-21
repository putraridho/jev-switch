//! Durations as short text for one terminal line.

const MINUTE_MS: u64 = 60_000;

/// "12m", "3h 5m", "2d 4h": short enough for one `/providers` line. Under a minute
/// reads "<1m" so a nearly finished cooldown is not shown as zero.
pub fn format_left(ms: u64) -> String {
    let minutes = ms / MINUTE_MS;
    match minutes {
        0 => "<1m".to_string(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_left_picks_a_readable_unit() {
        assert_eq!(format_left(0), "<1m");
        assert_eq!(format_left(59_999), "<1m");
        assert_eq!(format_left(12 * MINUTE_MS), "12m");
        assert_eq!(format_left(125 * MINUTE_MS), "2h 5m");
        assert_eq!(format_left((2 * 1440 + 4 * 60) * MINUTE_MS), "2d 4h");
    }
}
