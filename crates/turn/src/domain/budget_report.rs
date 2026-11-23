//! `/budget`'s account of the limits: paid money and subscription quota apart, each with what
//! kind of limit it is, and what attempts in flight have reserved.

use std::fmt::Write as _;

use jev_shared::ALL_PROVIDERS;

use super::budget::{short_count, Budget, Cap, Meter, Standing};
use super::usage_ledger::Reserved;

/// What the paid cap `cap` has used, over `meter` and `reserved`, in words.
fn cap_line(label: &str, cap: Option<Cap>, meter: &Meter, reserved: &Reserved) -> String {
    let (usd, tokens) = meter.paid();
    let spent = format!("${usd:.4} / {} tokens spent", short_count(tokens));
    let held = match reserved.unpriced {
        0 => format!("${:.4} / {} tokens reserved", reserved.paid_usd, short_count(reserved.paid_tokens)),
        n => format!("${:.4}+ / {} tokens reserved ({n} unpriced)", reserved.paid_usd, short_count(reserved.paid_tokens)),
    };
    let state = match (cap, Budget::standing_of(cap, meter)) {
        (None, _) => "no cap".to_string(),
        (Some(cap), Standing::Over) => format!("cap {}: reached or not checkable, paid models are refused", cap.describe()),
        (Some(cap), Standing::Close) => format!("cap {}: nearly used up", cap.describe()),
        (Some(cap), _) => format!("cap {}: ok", cap.describe()),
    };
    format!("  {label}: {spent}, {held}; {state}")
}

/// The limits section of `/budget`. `session` is the whole session's spend, `day` the day's
/// over every session.
pub fn limits_text(budget: &Budget, session: &Meter, reserved: &Reserved, day: &Meter) -> String {
    let mut out = String::new();
    out.push_str("  Paid money (hard: a paid attempt is refused if spent + reserved in flight + its own estimated maximum would pass a cap)\n");
    let _ = writeln!(out, "{}", cap_line("session", budget.cap, session, reserved));
    let _ = writeln!(out, "{}", cap_line("today  ", budget.daily, day, &Reserved::default()));
    out.push_str("  Subscription quota (soft: warns, never refuses; the plan's own limit is the provider's)\n");
    match budget.allowance {
        None => out.push_str("    No allowance set: /budget allowance 5m (tokens per subscription provider and session)\n"),
        Some(a) => {
            let _ = writeln!(out, "    Allowance {} tokens per provider and session:", short_count(a.tokens));
            for provider in ALL_PROVIDERS {
                let (used, unknown) = session.plan_use(provider);
                let held = reserved.plan_tokens.get(&provider).copied().unwrap_or(0);
                if used + held > 0 || unknown > 0 {
                    let note = if unknown > 0 { format!(" ({unknown} attempt(s) did not report)") } else { String::new() };
                    let _ = writeln!(out, "      {provider}: {} used{note}, {} reserved{}", short_count(used), short_count(held), if used >= a.tokens { "  past the allowance" } else { "" });
                }
            }
        }
    }
    let _ = write!(out, "  In flight: {} attempt(s) hold a reservation", reserved.attempts);
    out
}

#[cfg(test)]
mod tests;
