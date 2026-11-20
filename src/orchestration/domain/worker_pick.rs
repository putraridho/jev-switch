//! Picking a worker model for a brief. Pure: the only inputs are Jev's signals and the live
//! view of what is usable, so there is no way to ask for a particular model. Hard safety
//! rules (never Top tier, never Fable/Astra/Carbon, security work never to DeepSeek/Grok,
//! code-writing work never below Everyday) live here.

use crate::backend::ProviderId;
use crate::catalog::domain::model_table::{allowed_for, chain_with, models, pick, Allowances, ModelSpec, Overrides, TaskType, Tier, Usable};

use super::brief_signals::{worker_floor, worker_task, BriefSignals};

/// A provider whose plan is this much busier than the least busy candidate is passed
/// over for the next in Jev's chain: within the margin chain order wins (it is Jev's
/// quality judgment), beyond it the work is spread to protect the busy plan.
const SPREAD_SLACK: f64 = 0.10;
/// How much one worker already running counts as plan usage when spreading, so parallel
/// workers fan out across providers even before any provider reports its usage.
const LIVE_WORKER_LOAD: f64 = 0.15;

/// Workers are never Top tier (that is the planner's), and never the models kept for
/// very hard work (Fable, Astra, Carbon) whatever their tier says.
fn worker_eligible(spec: &ModelSpec, floor: Tier) -> bool {
    spec.tier != Tier::Top && !spec.very_hard_only && spec.tier >= floor
}

/// Why no worker can take a brief; the caller says so instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoWorker {
    pub task: TaskType,
    pub floor: Tier,
}

impl std::fmt::Display for NoWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let security = if self.task == TaskType::Security { " (security work never goes to deepseek or grok)" } else { "" };
        write!(
            f,
            "no usable worker model for {} work at {:?} tier or above{security}; workers are never top-tier models. \
             Do this piece yourself.",
            self.task, self.floor
        )
    }
}

impl std::error::Error for NoWorker {}

/// Everything [`pick_worker`] reads about the world; the caller owns the IO.
pub struct WorkerView<'a> {
    pub usable: &'a Usable,
    pub allow: &'a Allowances,
    pub overrides: &'a Overrides,
    /// Fraction of the plan used, as the provider last reported it.
    pub plan_utilization: &'a dyn Fn(ProviderId) -> Option<f64>,
    /// Workers currently running on the provider.
    pub live: &'a dyn Fn(ProviderId) -> usize,
    /// Rough size of the brief, so a model whose window is too small is skipped.
    pub context_tokens: u64,
}

/// Models Jev's chain for the task lists that may be workers, then every other eligible
/// model, strongest tier first. The tail is only used when the chain has no usable
/// worker (the Security and Review chains are all Top tier).
fn candidate_lists(task: TaskType, floor: Tier, overrides: &Overrides) -> [Vec<&'static ModelSpec>; 2] {
    let ok = |s: &&'static ModelSpec| worker_eligible(s, floor) && allowed_for(task, s);
    let chain: Vec<_> = chain_with(task, false, overrides).into_iter().filter(ok).collect();
    let mut rest: Vec<_> = models().iter().filter(ok).collect();
    rest.sort_by_key(|s| std::cmp::Reverse(s.tier));
    [chain, rest]
}

/// Among serving models (best first), one per provider at its best, prefer the least
/// busy provider; but a provider within [`SPREAD_SLACK`] of the least busy keeps its
/// chain position.
fn spread(serving: &[&'static ModelSpec], view: &WorkerView) -> &'static ModelSpec {
    let mut firsts: Vec<&'static ModelSpec> = Vec::new();
    for s in serving {
        if !firsts.iter().any(|f| f.provider == s.provider) {
            firsts.push(s);
        }
    }
    let load = |s: &ModelSpec| (view.plan_utilization)(s.provider).unwrap_or(0.0) + LIVE_WORKER_LOAD * (view.live)(s.provider) as f64;
    let least = firsts.iter().map(|s| load(s)).fold(f64::INFINITY, f64::min);
    firsts.iter().copied().find(|s| load(s) <= least + SPREAD_SLACK).unwrap_or(firsts[0])
}

/// The worker model for a brief. The only inputs are Jev's signals and the live view of
/// what is usable: there is no way to ask for a particular model.
pub fn pick_worker(signals: &BriefSignals, view: &WorkerView) -> Result<&'static ModelSpec, NoWorker> {
    let task = worker_task(signals);
    let floor = worker_floor(signals);
    for list in candidate_lists(task, floor, view.overrides) {
        let serving: Vec<_> = list
            .into_iter()
            .filter(|s| pick(&[*s], task, view.usable, view.allow, view.context_tokens).is_some())
            .collect();
        if !serving.is_empty() {
            return Ok(spread(&serving, view));
        }
    }
    Err(NoWorker { task, floor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};
    use std::collections::HashMap;

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

    fn providers_in(mask: u32) -> Vec<ProviderId> {
        PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect()
    }

    struct World {
        usable: Usable,
        allow: Allowances,
        overrides: Overrides,
        util: HashMap<ProviderId, f64>,
        live: HashMap<ProviderId, usize>,
    }

    impl World {
        fn new(ready: &[ProviderId]) -> Self {
            Self {
                usable: Usable::all_of(ready.iter().copied()),
                allow: Allowances { credits: true, per_token: true },
                overrides: Overrides::default(),
                util: HashMap::new(),
                live: HashMap::new(),
            }
        }

        fn pick(&self, s: &BriefSignals) -> Result<&'static ModelSpec, NoWorker> {
            let util = |p: ProviderId| self.util.get(&p).copied();
            let live = |p: ProviderId| self.live.get(&p).copied().unwrap_or(0);
            pick_worker(
                s,
                &WorkerView {
                    usable: &self.usable,
                    allow: &self.allow,
                    overrides: &self.overrides,
                    plan_utilization: &util,
                    live: &live,
                    context_tokens: 0,
                },
            )
        }
    }

    #[test]
    fn a_worker_is_never_top_tier_never_a_very_hard_model_and_always_usable() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let ready = providers_in(mask);
            let world = World::new(&ready);
            for task in TaskType::ALL {
                for read_only in [false, true] {
                    let s = BriefSignals { task, security_p: 0.1, edit_p: 0.1, read_only };
                    if let Ok(spec) = world.pick(&s) {
                        assert_ne!(spec.tier, Tier::Top, "{task} {ready:?}: {}", spec.id);
                        assert!(!spec.very_hard_only, "{}", spec.id);
                        assert!(ready.contains(&spec.provider), "unusable provider {} for {ready:?}", spec.provider);
                    }
                }
            }
        }
    }

    #[test]
    fn security_briefs_never_go_to_deepseek_or_grok() {
        for mask in 0..(1u32 << PROVIDERS.len()) {
            let world = World::new(&providers_in(mask));
            // Jev's task label, and Jev's security probability on an otherwise ordinary brief.
            for s in [
                BriefSignals::editing(TaskType::Security),
                BriefSignals { security_p: 0.95, ..BriefSignals::editing(TaskType::Feature) },
                BriefSignals { security_p: 0.95, read_only: true, edit_p: 0.0, ..BriefSignals::editing(TaskType::Explain) },
                BriefSignals::unjudged(),
            ] {
                if let Ok(spec) = world.pick(&s) {
                    assert!(!matches!(spec.provider, DeepSeek | Grok), "{:?} -> {} {}", s, spec.provider, spec.id);
                }
            }
        }
        // With only the barred providers ready there is no worker, rather than a bad one.
        let only_barred = World::new(&[DeepSeek, Grok]);
        assert_eq!(only_barred.pick(&BriefSignals::editing(TaskType::Security)), Err(NoWorker { task: TaskType::Security, floor: Tier::Everyday }));
    }

    #[test]
    fn code_never_drops_below_everyday_but_a_read_only_lookup_may_use_light() {
        let world = World::new(&PROVIDERS);
        for task in TaskType::ALL {
            let writes = world.pick(&BriefSignals::editing(task)).unwrap();
            assert!(writes.tier >= Tier::Everyday, "{task}: {}", writes.id);
            // The planner calling it read-only is not enough when Jev says it edits.
            let claims = BriefSignals { read_only: true, edit_p: 0.9, ..BriefSignals::editing(task) };
            assert!(world.pick(&claims).unwrap().tier >= Tier::Everyday, "{task}");
        }
        let lookup = BriefSignals { read_only: true, edit_p: 0.05, ..BriefSignals::editing(TaskType::Explain) };
        assert_eq!(world.pick(&lookup).unwrap().tier, Tier::Light, "Explain's chain starts with a Light model");
    }

    #[test]
    fn an_all_top_chain_falls_back_to_the_next_best_worker() {
        // Review's chain is Top models only; a review brief still gets an Everyday worker.
        let world = World::new(&[Claude]);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Review)).unwrap().id, "claude-sonnet-5-5");
    }

    #[test]
    fn chain_order_wins_until_a_provider_is_much_busier() {
        let mut world = World::new(&[Claude, Codex]);
        let first = world.pick(&BriefSignals::editing(TaskType::Feature)).unwrap();
        assert_eq!(first.provider, Claude, "Feature's chain starts with Sonnet");
        world.util.insert(Claude, 0.05);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Feature)).unwrap().provider, Claude, "within the margin");
        world.util.insert(Claude, 0.60);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Feature)).unwrap().provider, Codex, "claude's plan is much busier");
    }

    #[test]
    fn parallel_workers_fan_out_across_providers() {
        let mut world = World::new(&[Claude, Codex]);
        world.live.insert(Claude, 1);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Feature)).unwrap().provider, Codex, "one worker already on claude");
        world.live.insert(Codex, 1);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Feature)).unwrap().provider, Claude, "tied: chain order again");
    }

    #[test]
    fn spreading_never_picks_a_model_the_rules_bar() {
        // DeepSeek is idle and Claude is busy, but security still cannot use DeepSeek.
        let mut world = World::new(&[Claude, DeepSeek]);
        world.util.insert(Claude, 0.95);
        assert_eq!(world.pick(&BriefSignals::editing(TaskType::Security)).unwrap().provider, Claude);
    }
}
