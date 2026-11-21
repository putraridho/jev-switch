//! Stickiness within the chain's pick: keep the warm model, hold a downgrade, step down one tier.

use std::cmp::Reverse;

use super::pinned::can_serve;
use super::switch_gate::switch_gate;
use super::task::TaskSource;
use super::{Choice, RouteInput, RoutingState};
use crate::catalog::domain::model_table::{ModelSpec, Pick, TaskType};
use crate::judgment::domain::thresholds::MAX_HOLD;
use crate::turn::domain::cache;

/// Apply stickiness to the chain's best pick.
pub(super) fn sticky(
    state: &mut RoutingState,
    input: &RouteInput,
    task: TaskType,
    source: TaskSource,
    chain: &[&'static ModelSpec],
    best: Pick,
    current: Option<&'static ModelSpec>,
) -> Choice {
    let Some(cur) = current else {
        state.held = 0;
        let why = match &state.current {
            None => "first turn".to_string(),
            Some((_, id)) => format!("{id} cannot serve this turn any more"),
        };
        return Choice { spec: best.spec, why, dropped_tier: best.dropped_tier };
    };
    // The chain's entries on the current provider that can serve, best first.
    let own: Vec<_> = chain.iter().copied().filter(|s| s.provider == cur.provider && can_serve(s, task, input)).collect();
    if best.spec.provider == cur.provider {
        return hold_within(state, task, cur, &own, "same provider".into());
    }
    let Some(gate) = switch_gate(input, task, source) else {
        return stay(cur, "same task, warm session".into(), cur.tier < task.needed_tier());
    };
    let as_strong_here = own.first().is_some_and(|k| k.tier >= best.spec.tier);
    if !gate.confident && as_strong_here {
        let why = format!("not confident enough to switch from {} ({})", cur.provider, gate.why);
        return hold_within(state, task, cur, &own, why);
    }
    // A big warm cache is only given up for a better model or a forced reason, not for an
    // equally strong one elsewhere.
    if !gate.forced && as_strong_here && cache::worth_keeping(state.warm.as_ref(), cur.provider, input.now_ms) {
        let tokens = state.warm.map_or(0, |w| w.tokens);
        let why = format!("keeping {}: its {} token cache is warm and no stronger model is on offer", cur.provider, crate::shared::domain::wording::human_count(tokens));
        return hold_within(state, task, cur, &own, why);
    }
    state.held = 0;
    let why = format!("switched from {} ({})", cur.provider, gate.why);
    Choice { spec: best.spec, why, dropped_tier: best.dropped_tier }
}

pub(super) fn stay(spec: &'static ModelSpec, why: String, dropped_tier: bool) -> Choice {
    Choice { spec, why, dropped_tier }
}

/// Staying on a provider, the chain wants `own[0]` there (`own` is the chain's
/// servable entries of that provider, best first, never empty here). An upgrade is
/// immediate (quality matters now). A downgrade is held for up to `MAX_HOLD - 1`
/// consecutive turns, because dropping a tier rebuilds the cache and short dips in
/// difficulty are common; then it steps down ONE tier, not all the way.
fn hold_within(
    state: &mut RoutingState,
    task: TaskType,
    cur: &'static ModelSpec,
    own: &[&'static ModelSpec],
    note: String,
) -> Choice {
    let target = own[0];
    // Judged against what the task needs, like `catalog::pick`, not the chain head.
    let dropped = |spec: &'static ModelSpec| spec.tier < task.needed_tier();
    if target.tier >= cur.tier {
        // Equal tier is not worth rebuilding the cache for.
        let upgrade = target.tier > cur.tier;
        state.held = 0;
        let (spec, why) = if upgrade {
            (target, format!("upgrade within {}", cur.provider))
        } else {
            (cur, format!("same task, warm session ({note})"))
        };
        return Choice { spec, why, dropped_tier: dropped(spec) };
    }
    state.held += 1;
    if state.held < MAX_HOLD {
        let why = format!("holding {} for the cache, easier turn {}/{}", cur.id, state.held, MAX_HOLD - 1);
        return Choice { spec: cur, why, dropped_tier: dropped(cur) };
    }
    state.held = 0;
    let spec = one_step_down(cur, target, own);
    let why = format!("stepped down after {} easier turns", MAX_HOLD - 1);
    Choice { spec, why, dropped_tier: dropped(spec) }
}

/// The highest-tier entry of `own` that is still below `cur`: one step down, never
/// below `target` (which is `own[0]`, so the search always finds something) and never
/// a model the chain does not offer or the user cannot use.
fn one_step_down(cur: &'static ModelSpec, target: &'static ModelSpec, own: &[&'static ModelSpec]) -> &'static ModelSpec {
    own.iter().copied().filter(|m| m.tier < cur.tier).min_by_key(|m| Reverse(m.tier)).unwrap_or(target)
}
