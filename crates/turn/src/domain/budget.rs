//! What this session has spent, and the cap on the part that costs real money.
//!
//! Subscription models (Claude Max, ChatGPT, Antigravity) cost nothing per turn, so only
//! their tokens are counted. Models billed per token or from credits are "paid": a
//! [`Cap`] stops the router from picking them once the session has spent that much. A
//! price is never guessed: dollars show only for models listed in the price file
//! (`<state>/prices.txt`, lines of `model in cached out` in USD per million tokens), and
//! a paid model without a price counts tokens only.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use jev_backend::{ProviderId, Usage};
use jev_shared::Measurement;
use jev_catalog::domain::model_table as catalog;
use jev_catalog::domain::model_table::Billing;

const PRICE_FILE: &str = "prices.txt";
const BUDGET_FILE: &str = "budget";
const MILLION: f64 = 1_000_000.0;
/// A cap this close to used up starts to warn.
pub const WARN_AT: f64 = 0.8;

/// What a million tokens cost, in USD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Price {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

impl Price {
    /// The cost of `usage`; `input_tokens` counts the cached share too (see [`Usage`]).
    pub fn cost(&self, usage: &Usage) -> f64 {
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let fresh = usage.input_tokens - cached;
        (fresh as f64 * self.input + cached as f64 * self.cached_input + usage.output_tokens as f64 * self.output) / MILLION
    }

    /// What `tokens` of context cost to send again with no cache, over sending them cached.
    pub fn uncached_extra(&self, tokens: u64) -> f64 {
        tokens as f64 * (self.input - self.cached_input).max(0.0) / MILLION
    }
}

/// Prices by model id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceTable(BTreeMap<String, Price>);

impl PriceTable {
    /// Read a price file; lines that do not parse are returned so the caller can say so.
    pub fn parse(text: &str) -> (Self, Vec<usize>) {
        let mut table = BTreeMap::new();
        let mut bad = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            let numbers: Option<Vec<f64>> = words.get(1..).and_then(|w| w.iter().map(|n| n.parse::<f64>().ok().filter(|v| *v >= 0.0)).collect());
            match (words.first(), numbers.as_deref()) {
                (Some(model), Some(&[input, cached_input, output])) if words.len() == 4 => {
                    table.insert((*model).to_string(), Price { input, cached_input, output });
                }
                _ => bad.push(i + 1),
            }
        }
        (Self(table), bad)
    }

    /// The file in the state folder that holds the user's own prices.
    pub fn file_path(state_dir: &Path) -> std::path::PathBuf {
        state_dir.join(PRICE_FILE)
    }

    pub fn price(&self, model: &str) -> Option<&Price> {
        self.0.get(model)
    }

    pub fn cost(&self, model: &str, usage: &Usage) -> Option<f64> {
        (usage.measurement == Measurement::Reported).then(|| self.price(model).map(|p| p.cost(usage))).flatten()
    }
}

/// What one model has used in this session.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub turns: u32,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    /// Dollars, over the turns that had a price.
    pub usd: f64,
    /// Turns with no price in the table, so `usd` leaves them out.
    pub unpriced_turns: u32,
    pub unreported_turns: u32,
}

/// What the session has used, by model.
#[derive(Debug, Clone, Default)]
pub struct Meter {
    by_model: BTreeMap<(ProviderId, String), Spend>,
}

impl Meter {
    pub fn record(&mut self, provider: ProviderId, model: &str, usage: &Usage, prices: &PriceTable) {
        let s = self.by_model.entry((provider, model.to_string())).or_default();
        s.turns += 1;
        if usage.measurement == Measurement::Unavailable {
            s.unreported_turns += 1;
            return;
        }
        s.input_tokens += usage.input_tokens;
        s.cached_input_tokens += usage.cached_input_tokens;
        s.output_tokens += usage.output_tokens;
        match prices.cost(model, usage) {
            Some(c) => s.usd += c,
            None => s.unpriced_turns += 1,
        }
    }

    /// Add everything `other` recorded to this meter's own record.
    pub fn absorb(&mut self, other: &Meter) {
        for (key, s) in &other.by_model {
            let mine = self.by_model.entry(key.clone()).or_default();
            mine.turns += s.turns;
            mine.input_tokens += s.input_tokens;
            mine.cached_input_tokens += s.cached_input_tokens;
            mine.output_tokens += s.output_tokens;
            mine.usd += s.usd;
            mine.unpriced_turns += s.unpriced_turns;
            mine.unreported_turns += s.unreported_turns;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_model.is_empty()
    }

    pub fn rows(&self) -> impl Iterator<Item = (ProviderId, &str, &Spend)> {
        self.by_model.iter().map(|((p, m), s)| (*p, m.as_str(), s))
    }

    /// Dollars and tokens used by paid models (per-token API or credits).
    pub fn paid(&self) -> (f64, u64) {
        self.rows().filter(|(p, m, _)| is_paid(*p, m)).fold((0.0, 0), |(usd, tok), (_, _, s)| (usd + s.usd, tok + s.input_tokens + s.output_tokens))
    }

    /// Turns of paid models that have no price, so their dollars are unknown.
    pub fn unpriced_paid_turns(&self) -> u32 {
        self.rows().filter(|(p, m, _)| is_paid(*p, m)).map(|(_, _, s)| s.unpriced_turns).sum()
    }

    pub fn unreported_paid_turns(&self) -> u32 {
        self.rows().filter(|(provider, model, _)| is_paid(*provider, model)).map(|(_, _, spend)| spend.unreported_turns).sum()
    }

    pub fn clear(&mut self) {
        self.by_model.clear();
    }
}

/// Whether using the model costs money beyond the flat plan. A model the catalog does not
/// know counts as paid: the safe side of a cap.
pub fn is_paid(provider: ProviderId, model: &str) -> bool {
    catalog::find(provider, model).is_none_or(|m| m.billing != Billing::Subscription)
}

/// The limit on paid use for one session.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cap {
    Usd(f64),
    Tokens(u64),
}

impl Cap {
    /// `usd 5`, `tokens 2000000` (also `$5`, `2m tokens`, `500k tokens`).
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim().to_ascii_lowercase();
        let usage = "usage: usd <amount> | tokens <count> (like `usd 5` or `tokens 2m`)";
        let (unit, amount) = match t.split_once(char::is_whitespace) {
            // `2m tokens`, `500k tokens`, `5 usd`: the unit may come last.
            Some((a, u)) if matches!(u.trim(), "tokens" | "usd") => (u.trim(), a.trim()),
            Some((u, a)) => (u.trim(), a.trim()),
            None => return t.strip_prefix('$').map(Self::usd).unwrap_or_else(|| Err(usage.into())),
        };
        match unit {
            "usd" | "$" => Self::usd(amount),
            "tokens" => {
                let (digits, scale) = match amount.chars().last() {
                    Some('k') => (&amount[..amount.len() - 1], 1_000.0),
                    Some('m') => (&amount[..amount.len() - 1], 1_000_000.0),
                    _ => (amount, 1.0),
                };
                digits.parse::<f64>().ok().filter(|v| *v > 0.0).map(|v| Self::Tokens((v * scale) as u64)).ok_or_else(|| usage.into())
            }
            _ => Err(usage.into()),
        }
    }

    fn usd(amount: &str) -> Result<Self, String> {
        amount.trim().trim_start_matches('$').parse::<f64>().ok().filter(|v| *v > 0.0).map(Self::Usd).ok_or_else(|| "the amount must be a positive number".into())
    }

    fn used_fraction(&self, meter: &Meter) -> f64 {
        let (usd, tokens) = meter.paid();
        match self {
            Self::Usd(limit) => usd / limit,
            Self::Tokens(limit) => tokens as f64 / *limit as f64,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Usd(v) => format!("${v:.2}"),
            Self::Tokens(v) => format!("{} tokens", short_count(*v)),
        }
    }
}

/// Where the session stands against its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    NoCap,
    Under,
    /// Past [`WARN_AT`] of the cap.
    Close,
    Over,
}

/// The cap, if any, kept between sessions in `<state>/budget`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Budget {
    pub cap: Option<Cap>,
}

impl Budget {
    pub fn standing(&self, meter: &Meter) -> Standing {
        if self.cap.is_some() && meter.unreported_paid_turns() > 0 {
            return Standing::Over;
        }
        // A dollar cap cannot be checked against spend nobody priced (there is no built-in
        // price table), so it fails closed instead of staying silently at zero.
        if matches!(self.cap, Some(Cap::Usd(_))) && meter.unpriced_paid_turns() > 0 {
            return Standing::Over;
        }
        match self.cap.map(|c| c.used_fraction(meter)) {
            None => Standing::NoCap,
            Some(f) if f >= 1.0 => Standing::Over,
            Some(f) if f >= WARN_AT => Standing::Close,
            Some(_) => Standing::Under,
        }
    }

    /// Whether a paid model may still be picked.
    pub fn allows_paid(&self, meter: &Meter) -> bool {
        self.standing(meter) != Standing::Over
    }

    /// The file in the state folder that keeps the cap.
    pub fn file_path(state_dir: &Path) -> std::path::PathBuf {
        state_dir.join(BUDGET_FILE)
    }

    /// What the cap file holds, or `None` when there is no cap (and so no file).
    pub fn file_text(&self) -> Option<String> {
        match self.cap {
            Some(Cap::Usd(v)) => Some(format!("usd {v}")),
            Some(Cap::Tokens(v)) => Some(format!("tokens {v}")),
            None => None,
        }
    }
}

/// `12400` as `12.4k`, `3200000` as `3.2m`.
pub fn short_count(n: u64) -> String {
    let trim = |v: f64, unit: &str| {
        let s = format!("{v:.1}");
        format!("{}{unit}", s.strip_suffix(".0").unwrap_or(&s))
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => trim(n as f64 / 1_000.0, "k"),
        _ => trim(n as f64 / MILLION, "m"),
    }
}

/// `/budget`: tokens and dollars by model, plan use of the subscriptions, and the cap.
pub fn meter_text(meter: &Meter, budget: &Budget, plans: &[(String, f64)]) -> String {
    let mut out = String::new();
    if meter.is_empty() {
        out.push_str("  No model has answered yet in this session.\n");
    }
    for (provider, model, s) in meter.rows() {
        let cost = match (is_paid(provider, model), s.unpriced_turns) {
            (false, _) => "plan".to_string(),
            (true, 0) => format!("${:.4}", s.usd),
            (true, n) if n == s.turns => "no price set".to_string(),
            (true, _) => format!("${:.4}+ (some turns unpriced)", s.usd),
        };
        let count = |tokens| {
            if s.unreported_turns == s.turns {
                "unknown".to_string()
            } else if s.unreported_turns > 0 {
                format!(">={}", short_count(tokens))
            } else {
                short_count(tokens)
            }
        };
        let incomplete = if s.unreported_turns > 0 {
            format!("; consumption unknown for {} attempt(s)", s.unreported_turns)
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "  {:<9} {model:<20} {:>3} attempt(s)  in {:>7} ({} cached)  out {:>7}  {cost}{incomplete}",
            provider.as_str(),
            s.turns,
            count(s.input_tokens),
            count(s.cached_input_tokens),
            count(s.output_tokens)
        );
    }
    for (provider, used) in plans {
        let _ = writeln!(out, "  {provider} plan: {:.0}% of its limit used", used * 100.0);
    }
    let (usd, tokens) = meter.paid();
    if meter.unreported_paid_turns() > 0 {
        out.push_str("  Paid totals are incomplete: some attempts did not report consumption.\n");
    }
    match budget.cap {
        None => {
            let _ = writeln!(out, "  Paid use so far: ${usd:.4}, {} tokens. No cap set: /budget usd 5 or /budget tokens 2m", short_count(tokens));
        }
        Some(cap) => {
            let state = match budget.standing(meter) {
                Standing::Over if meter.unreported_paid_turns() > 0 => {
                    "cannot be checked: paid consumption is unknown, so paid models are no longer picked"
                }
                Standing::Over if matches!(cap, Cap::Usd(_)) && meter.unpriced_paid_turns() > 0 => {
                    "cannot be checked: a paid model has no price in prices.txt, so paid models are no longer picked (add its price, or use /budget tokens)"
                }
                Standing::Over => "reached: paid models are no longer picked",
                Standing::Close => "nearly used up",
                _ => "ok",
            };
            let _ = writeln!(out, "  Paid use so far: ${usd:.4}, {} tokens, cap {}: {state}", short_count(tokens), cap.describe());
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: u64, output: u64) -> Usage {
        Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: output, context_tokens: None, ..Usage::default() }
    }

    fn prices() -> PriceTable {
        PriceTable::parse("deepseek-v4-pro 1.0 0.1 4.0\n# c\n").0
    }

    #[test]
    fn prices_parse_and_bad_lines_are_numbered() {
        let (t, bad) = PriceTable::parse("a 1 0.5 2\nb 1 2\nc x y z\n\nd -1 1 1\n");
        assert_eq!(bad, vec![2, 3, 5]);
        assert_eq!(t.price("a"), Some(&Price { input: 1.0, cached_input: 0.5, output: 2.0 }));
        assert!(t.price("b").is_none());
    }

    #[test]
    fn cost_splits_cached_from_fresh_input() {
        let p = Price { input: 1.0, cached_input: 0.1, output: 4.0 };
        // 600k fresh + 400k cached + 100k out = 0.6 + 0.04 + 0.4
        let c = p.cost(&usage(1_000_000, 400_000, 100_000));
        assert!((c - 1.04).abs() < 1e-9, "{c}");
        assert!((p.uncached_extra(1_000_000) - 0.9).abs() < 1e-9);
        // cached above input cannot make fresh negative
        assert!(p.cost(&usage(10, 50, 0)) >= 0.0);
    }

    #[test]
    fn only_paid_models_count_toward_the_cap() {
        let mut m = Meter::default();
        let t = prices();
        m.record(ProviderId::Claude, "claude-opus-5-5", &usage(5_000_000, 0, 1_000_000), &t);
        assert_eq!(m.paid(), (0.0, 0));
        m.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage(1_000_000, 0, 250_000), &t);
        let (usd, tokens) = m.paid();
        assert!((usd - 2.0).abs() < 1e-9);
        assert_eq!(tokens, 1_250_000);
        // A paid model with no price counts tokens and flags the turn.
        m.record(ProviderId::DeepSeek, "deepseek-flash", &usage(100, 0, 100), &t);
        assert_eq!(m.rows().find(|(_, id, _)| *id == "deepseek-flash").map(|(_, _, s)| s.unpriced_turns), Some(1));
        assert!(is_paid(ProviderId::Claude, "claude-fable-5-2"));
        assert!(is_paid(ProviderId::Claude, "not-a-model"));
    }

    #[test]
    fn unknown_paid_consumption_is_not_free_and_makes_either_cap_fail_closed() {
        let prices = PriceTable::parse("deepseek-flash 1 0.1 2").0;
        let unknown = Usage { measurement: Measurement::Unavailable, ..Usage::default() };
        let mut meter = Meter::default();
        meter.record(ProviderId::DeepSeek, "deepseek-flash", &unknown, &prices);
        assert_eq!(meter.unreported_paid_turns(), 1);
        assert_eq!(prices.cost("deepseek-flash", &unknown), None);
        for cap in [Cap::Usd(5.0), Cap::Tokens(100)] {
            let budget = Budget { cap: Some(cap) };
            assert_eq!(budget.standing(&meter), Standing::Over);
            assert!(!budget.allows_paid(&meter));
            let text = meter_text(&meter, &budget, &[]);
            assert!(text.contains("consumption unknown for 1 attempt(s)"), "{text}");
            assert!(text.contains("Paid totals are incomplete") && text.contains("cannot be checked"), "{text}");
        }
    }

    #[test]
    fn unknown_subscription_consumption_does_not_close_paid_caps() {
        let mut meter = Meter::default();
        let unknown = Usage { measurement: Measurement::Unavailable, ..Usage::default() };
        meter.record(ProviderId::Claude, catalog::MODEL_MID, &unknown, &PriceTable::default());
        assert_eq!(meter.unreported_paid_turns(), 0);
        assert!(Budget { cap: Some(Cap::Tokens(100)) }.allows_paid(&meter));
        assert!(meter_text(&meter, &Budget::default(), &[]).contains("consumption unknown for 1 attempt(s)"));
    }

    #[test]
    fn a_reported_zero_remains_known_and_mixed_totals_are_marked_as_lower_bounds() {
        let prices = PriceTable::parse("deepseek-flash 1 0.1 2").0;
        let mut meter = Meter::default();
        meter.record(ProviderId::DeepSeek, "deepseek-flash", &Usage::default(), &prices);
        assert_eq!(meter.unreported_paid_turns(), 0);
        assert!(Budget { cap: Some(Cap::Tokens(100)) }.allows_paid(&meter));
        meter.record(ProviderId::DeepSeek, "deepseek-flash", &usage(20, 10, 5), &prices);
        meter.record(ProviderId::DeepSeek, "deepseek-flash", &Usage { measurement: Measurement::Unavailable, ..Usage::default() }, &prices);
        let text = meter_text(&meter, &Budget::default(), &[]);
        assert!(text.contains(">=20") && text.contains(">=5"), "{text}");
    }

    #[test]
    fn caps_parse() {
        assert_eq!(Cap::parse("usd 5"), Ok(Cap::Usd(5.0)));
        assert_eq!(Cap::parse("$2.5"), Ok(Cap::Usd(2.5)));
        assert_eq!(Cap::parse("tokens 2m"), Ok(Cap::Tokens(2_000_000)));
        assert_eq!(Cap::parse("tokens 500k"), Ok(Cap::Tokens(500_000)));
        assert_eq!(Cap::parse("tokens 1234"), Ok(Cap::Tokens(1234)));
        assert_eq!(Cap::parse("2m tokens"), Ok(Cap::Tokens(2_000_000)));
        assert_eq!(Cap::parse("500k tokens"), Ok(Cap::Tokens(500_000)));
        assert_eq!(Cap::parse("5 usd"), Ok(Cap::Usd(5.0)));
        for bad in ["", "usd", "usd -1", "usd x", "tokens 0", "pounds 3"] {
            assert!(Cap::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn standing_follows_the_cap() {
        let t = prices();
        let mut m = Meter::default();
        let b = Budget { cap: Some(Cap::Usd(10.0)) };
        assert_eq!(Budget::default().standing(&m), Standing::NoCap);
        assert_eq!(b.standing(&m), Standing::Under);
        m.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage(8_000_000, 0, 0), &t); // $8
        assert_eq!(b.standing(&m), Standing::Close);
        assert!(b.allows_paid(&m));
        m.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage(2_000_000, 0, 0), &t); // $10
        assert_eq!(b.standing(&m), Standing::Over);
        assert!(!b.allows_paid(&m));
        assert!(Budget::default().allows_paid(&m));
        m.clear();
        assert!(b.allows_paid(&m));
    }

    #[test]
    fn a_dollar_cap_is_not_silently_inert_without_prices() {
        let mut m = Meter::default();
        let b = Budget { cap: Some(Cap::Usd(5.0)) };
        m.record(ProviderId::DeepSeek, "deepseek-flash", &usage(1_000_000, 0, 1_000_000), &PriceTable::default());
        assert_eq!(b.standing(&m), Standing::Over);
        assert!(!b.allows_paid(&m));
        assert!(meter_text(&m, &b, &[]).contains("cannot be checked"));
        // A tokens cap needs no price.
        assert_eq!(Budget { cap: Some(Cap::Tokens(10_000_000)) }.standing(&m), Standing::Under);
    }

    #[test]
    fn the_meter_screen() {
        let t = prices();
        let mut m = Meter::default();
        assert!(meter_text(&m, &Budget::default(), &[]).contains("No model has answered"));
        m.record(ProviderId::Claude, "claude-opus-5-5", &usage(12_400, 10_000, 800), &t);
        m.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage(1_000_000, 0, 0), &t);
        let s = meter_text(&m, &Budget { cap: Some(Cap::Usd(1.0)) }, &[("claude".into(), 0.42)]);
        assert!(s.contains("claude    claude-opus-5-5") && s.contains("in   12.4k (10k cached)") && s.contains("plan"), "{s}");
        assert!(s.contains("$1.0000"), "{s}");
        assert!(s.contains("claude plan: 42% of its limit used"), "{s}");
        assert!(s.contains("cap $1.00: reached: paid models are no longer picked"), "{s}");
        assert!(meter_text(&m, &Budget::default(), &[]).contains("No cap set"));
    }

    #[test]
    fn short_counts() {
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(12_400), "12.4k");
        assert_eq!(short_count(10_000), "10k");
        assert_eq!(short_count(3_200_000), "3.2m");
    }
}
