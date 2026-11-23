//! Whether an attempt may start, against the limits of the [`Budget`].
//!
//! Two different limits, stated as what they are:
//!
//! - **Paid money** (the session cap and the daily cap) is *hard*. An attempt that costs money
//!   is admitted only if what was spent, plus what other attempts have reserved in flight,
//!   plus this attempt's own estimated maximum stays within the cap. Spend nobody reported, or
//!   a model with no price under a dollar cap, fails closed. The cap is checked at the start of
//!   an attempt: one that runs over its estimate is still counted when it settles, but it is
//!   not cut off mid-way.
//! - **Subscription quota** (the allowance) is *soft*. The provider enforces the plan's real
//!   limit and reports it late and roughly, so an attempt past the allowance is admitted with a
//!   warning, never refused.

use jev_backend::{ProviderId, Usage};

use super::budget::{short_count, Budget, Cap, Meter, PriceTable};
use super::usage_ledger::Reserved;

/// What an attempt can use at most, as far as anyone can tell beforehand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub tokens: u64,
    /// `None` when the model has no price.
    pub usd: Option<f64>,
}

/// Output a model may produce in one attempt, assumed when reserving: the CLIs give no hard
/// output limit, so this is an assumption, not a guarantee.
pub const OUTPUT_ALLOWANCE_TOKENS: u64 = 16_000;

impl Estimate {
    /// Input of `input_tokens` sent uncached, and [`OUTPUT_ALLOWANCE_TOKENS`] back.
    pub fn of(model: &str, input_tokens: u64, prices: &PriceTable) -> Self {
        let usage = Usage { input_tokens, output_tokens: OUTPUT_ALLOWANCE_TOKENS, ..Usage::default() };
        Self { tokens: input_tokens + OUTPUT_ALLOWANCE_TOKENS, usd: prices.price(model).map(|p| p.cost(&usage)) }
    }
}

/// The attempt that asks to start.
#[derive(Debug, Clone, Copy)]
pub struct Attempt<'a> {
    pub provider: ProviderId,
    pub paid: bool,
    pub estimate: Estimate,
    pub budget: &'a Budget,
}

/// What the ledger says now: spend and reservations, for the session and for the day.
pub struct Standing<'a> {
    pub session: &'a Meter,
    pub session_reserved: &'a Reserved,
    pub day: &'a Meter,
    pub day_reserved: &'a Reserved,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    Admitted { warning: Option<String> },
    Refused(String),
}

pub fn admit(attempt: &Attempt, now: &Standing) -> Admission {
    if attempt.paid {
        for (label, cap, meter, reserved) in [("session", attempt.budget.cap, now.session, now.session_reserved), ("daily", attempt.budget.daily, now.day, now.day_reserved)] {
            if let Some(cap) = cap {
                if let Err(why) = within(label, cap, meter, reserved, &attempt.estimate) {
                    return Admission::Refused(why);
                }
            }
        }
        return Admission::Admitted { warning: None };
    }
    Admission::Admitted { warning: attempt.budget.allowance.and_then(|a| allowance_warning(a.tokens, attempt, now)) }
}

/// `Ok` when this attempt's estimate fits under `cap` beside what is spent and reserved.
fn within(label: &str, cap: Cap, meter: &Meter, reserved: &Reserved, estimate: &Estimate) -> Result<(), String> {
    if meter.unreported_paid_turns() > 0 {
        return Err(format!("the {label} paid use cannot be checked: an attempt did not report its consumption, so paid models are not admitted"));
    }
    let (usd, tokens) = meter.paid();
    match cap {
        Cap::Usd(limit) => {
            if meter.unpriced_paid_turns() > 0 || reserved.unpriced > 0 || estimate.usd.is_none() {
                return Err(format!("the {label} dollar cap cannot be checked: a paid model has no price in prices.txt (add its price, or use a tokens cap)"));
            }
            let (held, extra) = (reserved.paid_usd, estimate.usd.unwrap_or_default());
            if usd + held >= limit || usd + held + extra > limit {
                return Err(format!("the {label} cap ${limit:.2} would be passed: ${usd:.2} spent, ${held:.2} reserved by attempts in flight, ${extra:.2} estimated for this one"));
            }
        }
        Cap::Tokens(limit) => {
            let (held, extra) = (reserved.paid_tokens, estimate.tokens);
            if tokens + held >= limit || tokens + held + extra > limit {
                return Err(format!(
                    "the {label} cap {} would be passed: {} spent, {} reserved by attempts in flight, {} estimated for this one",
                    cap.describe(),
                    short_count(tokens),
                    short_count(held),
                    short_count(extra)
                ));
            }
        }
    }
    Ok(())
}

fn allowance_warning(limit: u64, attempt: &Attempt, now: &Standing) -> Option<String> {
    let (used, unknown) = now.session.plan_use(attempt.provider);
    let reserved = now.session_reserved.plan_tokens.get(&attempt.provider).copied().unwrap_or(0);
    let total = used + reserved + attempt.estimate.tokens;
    (total > limit || (unknown > 0 && used >= limit)).then(|| {
        format!(
            "{} plan allowance {} is {}: {} used{}, {} reserved, {} estimated (a soft limit: it warns, the plan's own limit decides)",
            attempt.provider,
            short_count(limit),
            if used >= limit { "used up" } else { "about to be passed" },
            short_count(used),
            if unknown > 0 { " (some attempts did not report)" } else { "" },
            short_count(reserved),
            short_count(attempt.estimate.tokens)
        )
    })
}

#[cfg(test)]
mod tests;
