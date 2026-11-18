//! The quota fuel gauge: how much of a plan window is used, as a bar, and the nudge to
//! switch provider before the limit hits.

/// How worried to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuelLevel {
    Ok,
    /// 80% used or more.
    Low,
    /// 95% used or more.
    Critical,
}

/// A drawn gauge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gauge {
    pub bar: String,
    pub percent: u32,
    pub level: FuelLevel,
}

pub const LOW_AT: f64 = 0.8;
pub const CRITICAL_AT: f64 = 0.95;

/// `used` (0..1, clamped; NaN counts as 0) as a bar of `cells` cells: `▰▰▰▱▱`.
pub fn fuel_gauge(used: f64, cells: usize) -> Gauge {
    let used = if used.is_nan() { 0.0 } else { used.clamp(0.0, 1.0) };
    let filled = ((used * cells as f64).round() as usize).min(cells);
    let level = if used >= CRITICAL_AT {
        FuelLevel::Critical
    } else if used >= LOW_AT {
        FuelLevel::Low
    } else {
        FuelLevel::Ok
    };
    Gauge { bar: format!("{}{}", "▰".repeat(filled), "▱".repeat(cells - filled)), percent: (used * 100.0).round() as u32, level }
}

/// One provider's fuel, for choosing where to switch.
#[derive(Debug, Clone, PartialEq)]
pub struct Fuel {
    pub provider: String,
    pub used: f64,
    pub ready: bool,
}

/// The warning shown before a limit hits, with the roomiest other ready provider to switch
/// to: `claude is 92% used. Press Ctrl-O to switch to codex (31% used)`. `None` while the
/// current provider has room; when nothing else is ready the warning says so.
pub fn switch_suggestion(current: &Fuel, others: &[Fuel]) -> Option<String> {
    if current.used < LOW_AT {
        return None;
    }
    let pct = |f: f64| (f.clamp(0.0, 1.0) * 100.0).round() as u32;
    let best = others.iter().filter(|o| o.ready && o.provider != current.provider).min_by(|a, b| a.used.total_cmp(&b.used));
    Some(match best {
        Some(o) => format!("{} is {}% used. Press Ctrl-O to switch to {} ({}% used)", current.provider, pct(current.used), o.provider, pct(o.used)),
        None => format!("{} is {}% used and no other provider is ready", current.provider, pct(current.used)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_table() {
        for (used, cells, bar, pct, level) in [
            (0.0, 5, "▱▱▱▱▱", 0, FuelLevel::Ok),
            (0.6, 5, "▰▰▰▱▱", 60, FuelLevel::Ok),
            (0.8, 5, "▰▰▰▰▱", 80, FuelLevel::Low),
            (0.95, 5, "▰▰▰▰▰", 95, FuelLevel::Critical),
            (1.0, 5, "▰▰▰▰▰", 100, FuelLevel::Critical),
            (7.0, 4, "▰▰▰▰", 100, FuelLevel::Critical),
            (-1.0, 4, "▱▱▱▱", 0, FuelLevel::Ok),
            (f64::NAN, 2, "▱▱", 0, FuelLevel::Ok),
            (0.5, 0, "", 50, FuelLevel::Ok),
        ] {
            let g = fuel_gauge(used, cells);
            assert_eq!((g.bar.as_str(), g.percent, g.level), (bar, pct, level), "{used}");
        }
    }

    #[test]
    fn suggestion_picks_the_roomiest_ready_provider() {
        let f = |p: &str, used: f64, ready: bool| Fuel { provider: p.into(), used, ready };
        let cur = f("claude", 0.92, true);
        let others = [f("claude", 0.1, true), f("codex", 0.31, true), f("gemini", 0.05, false), f("grok", 0.5, true)];
        assert_eq!(switch_suggestion(&cur, &others).unwrap(), "claude is 92% used. Press Ctrl-O to switch to codex (31% used)");
        assert_eq!(switch_suggestion(&cur, &[]).unwrap(), "claude is 92% used and no other provider is ready");
        assert_eq!(switch_suggestion(&f("claude", 0.5, true), &others), None);
    }
}
