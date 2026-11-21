//! Session-state checks: extra-credit models, cooldowns, blocked providers.

use crate::providers::domain::availability::State;
use crate::shared::ProviderId;

use super::{check, steps, Check, Inputs, Severity};

/// Claude is ready, extra-credit models are off, and the plan is not Max (Max
/// includes them), so say why Fable-class models are skipped and how to opt in.
pub(super) fn credits_check(inputs: &Inputs) -> Option<Check> {
    if inputs.allow_credits {
        return None;
    }
    let claude = inputs.avail.iter().find(|a| a.provider == ProviderId::Claude && a.state == State::Ready)?;
    if claude.plan.as_deref().is_some_and(|p| p.to_ascii_lowercase().contains("max")) {
        return None;
    }
    Some(check(
        "credits",
        "Extra-credit models are skipped",
        Severity::Info,
        "Models billed as extra credits on your Claude plan (such as Fable) are never picked unless you opt in.",
        steps(&["export JEV_ALLOW_CREDITS=1"]),
    ))
}

pub(super) fn cooldown_check(lines: &[String]) -> Option<Check> {
    (!lines.is_empty()).then(|| {
        check("cooldowns", "Providers are cooling down", Severity::Info, lines.join("\n"), steps(&["Nothing to do: they come back on their own when the limit resets"]))
    })
}

pub(super) fn blocked_check(blocked: &[(ProviderId, String)]) -> Option<Check> {
    (!blocked.is_empty()).then(|| {
        let detail = blocked.iter().map(|(p, why)| format!("{p}: {why}")).collect::<Vec<_>>().join("\n");
        check("blocked", "Providers are skipped for this session", Severity::Info, detail, Vec::new())
    })
}
