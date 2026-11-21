//! The savings ticker and the `/wrapped` recap: what routing saved against sending every
//! turn to the most expensive model.

use std::collections::BTreeMap;

/// What one turn cost, and what it would have cost on the baseline (top) model.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnCost {
    /// Local calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub provider: String,
    pub cost_usd: f64,
    pub baseline_usd: f64,
    /// Share of a subscription window the turn used (0..1), when it ran on one.
    pub quota_used: f64,
    pub baseline_quota: f64,
}

/// Totals over some turns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Savings {
    pub turns: usize,
    pub spent_usd: f64,
    pub baseline_usd: f64,
    pub quota_used: f64,
    pub baseline_quota: f64,
}

impl Savings {
    pub fn of(turns: &[TurnCost]) -> Self {
        turns.iter().fold(Self::default(), |mut s, t| {
            s.turns += 1;
            s.spent_usd += t.cost_usd;
            s.baseline_usd += t.baseline_usd;
            s.quota_used += t.quota_used;
            s.baseline_quota += t.baseline_quota;
            s
        })
    }

    /// Dollars saved; never negative (routing that cost more is not "negative savings").
    pub fn saved_usd(&self) -> f64 {
        (self.baseline_usd - self.spent_usd).max(0.0)
    }

    /// The share of the baseline quota not used, 0..1.
    pub fn quota_saved_fraction(&self) -> f64 {
        if self.baseline_quota <= 0.0 {
            return 0.0;
        }
        ((self.baseline_quota - self.quota_used) / self.baseline_quota).clamp(0.0, 1.0)
    }
}

fn dollars(x: f64) -> String {
    format!("${x:.2}")
}

/// The footer ticker: `saved $1.20 · 38% of quota`, empty until something was saved.
pub fn ticker(s: &Savings) -> String {
    let quota = (s.quota_saved_fraction() * 100.0).round() as u32;
    match (s.saved_usd() >= 0.005, quota > 0) {
        (true, true) => format!("saved {} · {quota}% of quota", dollars(s.saved_usd())),
        (true, false) => format!("saved {}", dollars(s.saved_usd())),
        (false, true) => format!("saved {quota}% of quota"),
        (false, false) => String::new(),
    }
}

/// `/wrapped` for one day: turns, spend, savings and the providers used, most used first.
/// Another day's turns are ignored.
pub fn wrapped(day: &str, turns: &[TurnCost]) -> String {
    let today: Vec<TurnCost> = turns.iter().filter(|t| t.day == day).cloned().collect();
    if today.is_empty() {
        return format!("{day}: no turns yet.");
    }
    let s = Savings::of(&today);
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &today {
        *by.entry(&t.provider).or_default() += 1;
    }
    let mut used: Vec<(&str, usize)> = by.into_iter().collect();
    used.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let who: Vec<String> = used.iter().map(|(p, n)| format!("{p} {n}")).collect();
    let tick = ticker(&s);
    let mut out = format!("{day}: {} turn{}, spent {}", s.turns, if s.turns == 1 { "" } else { "s" }, dollars(s.spent_usd));
    if !tick.is_empty() {
        out.push_str(&format!(", {tick}"));
    }
    out.push_str(&format!("\nProviders: {}", who.join(", ")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(day: &str, p: &str, cost: f64, base: f64, q: f64, bq: f64) -> TurnCost {
        TurnCost { day: day.into(), provider: p.into(), cost_usd: cost, baseline_usd: base, quota_used: q, baseline_quota: bq }
    }

    #[test]
    fn ticker_forms() {
        let s = Savings::of(&[t("d", "codex", 0.3, 1.5, 0.1, 0.4)]);
        assert_eq!(ticker(&s), "saved $1.20 · 75% of quota");
        assert_eq!(ticker(&Savings::of(&[t("d", "c", 0.3, 1.5, 0.0, 0.0)])), "saved $1.20");
        assert_eq!(ticker(&Savings::of(&[t("d", "c", 0.0, 0.0, 0.1, 0.4)])), "saved 75% of quota");
        assert_eq!(ticker(&Savings::default()), "");
        assert_eq!(ticker(&Savings::of(&[t("d", "c", 2.0, 1.0, 0.5, 0.2)])), "", "never negative");
    }

    #[test]
    fn wrapped_summarises_one_day() {
        let turns = [
            t("2026-11-18", "codex", 0.1, 0.5, 0.0, 0.0),
            t("2026-11-18", "claude", 0.4, 0.5, 0.0, 0.0),
            t("2026-11-18", "codex", 0.1, 0.5, 0.0, 0.0),
            t("2026-11-17", "grok", 9.0, 9.0, 0.0, 0.0),
        ];
        assert_eq!(wrapped("2026-11-18", &turns), "2026-11-18: 3 turns, spent $0.60, saved $0.90\nProviders: codex 2, claude 1");
        assert_eq!(wrapped("2026-11-19", &turns), "2026-11-19: no turns yet.");
        assert!(wrapped("2026-11-17", &turns).starts_with("2026-11-17: 1 turn, spent $9.00\n"));
    }
}
