//! Pinned models and providers: served exactly or reported, never swapped.

use super::{Choice, NoRoute, RouteInput};
use jev_catalog::domain::model_table::{ModelSpec, TaskType, allowed_for, find, models, pick};
use jev_shared::ProviderId;

/// Whether `spec` may serve this turn: usable, billing-allowed, allowed for the task
/// and big enough for the session. Delegates to [`pick`] so the one definition of
/// "may serve" lives in the catalog and cannot drift from it.
pub(super) fn can_serve(spec: &'static ModelSpec, task: TaskType, input: &RouteInput) -> bool {
    pick(&[spec], task, input.usable, input.allow, input.context_tokens).is_some()
}

/// Plain-words reason a model cannot serve, checked in [`pick`]'s order.
pub(super) fn why_unusable(spec: &ModelSpec, task: TaskType, input: &RouteInput) -> &'static str {
    if !input.usable.contains(spec) {
        "not logged in or not entitled"
    } else if !allowed_for(task, spec) {
        "barred for this kind of work"
    } else if !input.usable.fits( spec, input.context_tokens) {
        "the session is larger than its context window"
    } else {
        "its billing (credits or API key) is not enabled"
    }
}

/// A pinned model is used exactly or not at all.
pub(super) fn pin_model(provider: ProviderId, id: &str, input: &RouteInput, task: TaskType) -> Result<Choice, NoRoute> {
    let spec = find(provider, id)
        .ok_or_else(|| NoRoute::PinUnusable { what: format!("pinned model {provider}:{id} is not a known model") })?;
    if !can_serve(spec, task, input) {
        let why = why_unusable(spec, task, input);
        return Err(NoRoute::PinUnusable { what: format!("pinned model {provider}:{id} cannot be used: {why}") });
    }
    Ok(Choice { spec, why: "pinned model".into(), dropped_tier: false })
}

/// A pinned provider serves with its best model for the task: the task's chain order
/// first, then the rest of its catalog so the pin still works where the chain has no
/// entry for that provider.
pub(super) fn pin_provider(provider: ProviderId, input: &RouteInput, task: TaskType, chain: &[&'static ModelSpec]) -> Result<Choice, NoRoute> {
    let candidates: Vec<_> = chain
        .iter()
        .copied()
        .filter(|s| s.provider == provider)
        .chain(models().iter().filter(|m| m.provider == provider && !m.very_hard_only))
        .collect();
    let picked = pick(&candidates, task, input.usable, input.allow, input.context_tokens)
        .ok_or_else(|| NoRoute::PinUnusable { what: format!("pinned provider {provider} has no usable model for {task} work") })?;
    Ok(Choice { spec: picked.spec, why: format!("pinned provider {provider}"), dropped_tier: false })
}
