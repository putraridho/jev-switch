//! Automatic routing: the chain's best pick, steered off a nearly spent Claude plan.

use super::pinned::can_serve;
use super::stickiness::{stay, sticky};
use super::task::TaskSource;
use super::{Choice, NoRoute, RouteInput, RoutingState};
use jev_catalog::domain::model_table::{ModelSpec, TaskType, pick};
use jev_shared::ProviderId;

/// Claude plan utilisation from which work is steered to other providers, leaving
/// the last of the quota for when Claude is the only option.
const PLAN_STEER_AT: f64 = 0.90;

pub(super) fn auto_choice(
    state: &mut RoutingState,
    input: &RouteInput,
    task: TaskType,
    source: TaskSource,
    chain: &[&'static ModelSpec],
) -> Result<Choice, NoRoute> {
    let current = state.current_spec().filter(|c| can_serve(c, task, input));
    // No judgment means no evidence the work changed: stay on a model that works.
    if let (TaskSource::NoSignals, Some(cur)) = (source, current) {
        return Ok(stay(cur, "Jev unavailable, keeping the current model".into(), false));
    }
    let steer = plan_steered(chain, task, input);
    let chain = steer.chain.as_deref().unwrap_or(chain);
    let best = pick(chain, task, input.usable, input.allow, input.context_tokens)
        .ok_or(NoRoute::NothingUsable { task })?;
    let mut choice = sticky(state, input, task, source, chain, best, current);
    if let Some(note) = steer.note.filter(|_| choice.spec.provider != ProviderId::Claude) {
        choice.why = format!("{}; {note}", choice.why);
    }
    Ok(choice)
}

/// The chain minus Claude, when Claude's plan is nearly used up and the work can go
/// elsewhere. If nothing else can serve, `chain` is `None` and Claude still serves,
/// so a nearly-empty plan never leaves the user without an answer.
struct Steer {
    chain: Option<Vec<&'static ModelSpec>>,
    note: Option<String>,
}

fn plan_steered(chain: &[&'static ModelSpec], task: TaskType, input: &RouteInput) -> Steer {
    let none = Steer { chain: None, note: None };
    let Some(used) = (input.plan_utilization)(ProviderId::Claude).filter(|u| *u >= PLAN_STEER_AT) else {
        return none;
    };
    let others: Vec<_> = chain.iter().copied().filter(|s| s.provider != ProviderId::Claude).collect();
    if pick(&others, task, input.usable, input.allow, input.context_tokens).is_none() {
        return none;
    }
    Steer { chain: Some(others), note: Some(format!("steered off claude (plan {:.0}% used)", used * 100.0)) }
}
