//! Which model answers this turn, given what Jev said and what the session is doing.
//!
//! [`catalog`] knows which models exist and which are usable; Jev says what kind of
//! task a turn is. This module is the policy in between: it turns those inputs into
//! ONE model for the turn, and a plain-words reason for the `[route]` line.
//!
//! It is pure on purpose. Every input (Jev's signals, the pin, usability, billing,
//! plan utilisation) is passed in and the only memory is [`RoutingState`], so each
//! rule is a unit test and the caller owns all IO, clocks and env reads.
//!
//! The thing the rules protect is the warm session. Switching provider costs a cold
//! session plus a handoff, so a provider change only happens when the evidence says
//! the work really changed (see [`switch_gate`]); within a provider an upgrade is
//! immediate (quality) but a downgrade is delayed (cache), see [`hold_within`].

use std::cmp::Reverse;

use crate::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::backend::ProviderId;
use crate::cache::{self, WarmCache};
use crate::catalog::{
    Allowances, ModelSpec, Overrides, Pick, TaskType, Usable, allowed_for, chain_with, find, models, pick,
};
use crate::router::{Orchestration, CODE_EDIT_P, CODE_SECURITY_P, MAX_HOLD, ProviderWant, Security, TOP_THRESHOLD};

/// Jev says a new task is a clear provider-switch candidate only from this
/// `task_confidence`; below it the turn is too vague to justify a cold session.
const MIN_BOUNDARY_CONFIDENCE: f64 = 0.5;
/// A turn "continues" the previous work when `continues_p` reaches this; below it the
/// turn is a new task.
const CONTINUES_P: f64 = 0.5;
/// At or below this many words a prompt is a short, context-dependent follow-up
/// ("go on", "why?"). Mirrors [`crate::router::is_short_follow_up`], which takes text
/// rather than the word count this pure module is handed; a test keeps them equal.
const SHORT_FOLLOW_UP_WORDS: usize = 8;
/// Below this task confidence a boundary keeps the current provider when it already
/// has an equally strong model: not worth a cold session on a shaky judgment.
const CONFIDENT_SWITCH: f64 = 0.7;
/// A correction this likely means the current provider may itself be the problem.
const CORRECTION_P: f64 = 0.7;
/// Claude plan utilisation from which work is steered to other providers, leaving
/// the last of the quota for when Claude is the only option.
const PLAN_STEER_AT: f64 = 0.90;

/// What `/model` pinned. A pin is never silently overridden.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Pin {
    /// Route automatically.
    #[default]
    Auto,
    /// Any model of this provider; the best one for the task is chosen.
    Provider(ProviderId),
    /// Exactly this model.
    Model(ProviderId, String),
}

/// Who set the pin. A person's own pin is never overridden; a pin that came from the repo's
/// team file is a default that the person's routing rules (`avoid`, `only`) still bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinOrigin {
    #[default]
    User,
    Team,
}

impl Pin {
    /// Read the argument of `/model`: `auto`, a provider (`codex`), one model
    /// (`codex:gpt-6.1-sol`), or the old Claude aliases `sonnet|opus|haiku`. The one
    /// place the syntax lives; a model id must exist in the catalog, so a typo is
    /// reported instead of pinning something that can never run.
    pub fn parse(arg: &str) -> Result<Pin, String> {
        let arg = arg.trim().to_ascii_lowercase();
        let alias = |id: &str| Ok(Pin::Model(ProviderId::Claude, id.to_string()));
        match arg.as_str() {
            "auto" => Ok(Pin::Auto),
            "haiku" => alias(MODEL_CHEAP),
            "sonnet" => alias(MODEL_MID),
            "opus" => alias(MODEL_STRONG),
            _ => match arg.split_once(':') {
                None => arg.parse().map(Pin::Provider),
                Some((provider, model)) => {
                    let provider: ProviderId = provider.parse()?;
                    if find(provider, model).is_none() {
                        let known: Vec<_> = models().iter().filter(|m| m.provider == provider).map(|m| m.id).collect();
                        return Err(format!("unknown model `{model}` for {provider} (known: {})", known.join(", ")));
                    }
                    Ok(Pin::Model(provider, model.to_string()))
                }
            },
        }
    }
}

impl std::fmt::Display for Pin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Provider(p) => write!(f, "{p}"),
            Self::Model(p, id) => write!(f, "{p}:{id}"),
        }
    }
}

/// Everything Jev judged about the turn. The caller builds it from Jev's answers and
/// the keyword scan, so this module never calls Jev or reads the prompt.
#[derive(Debug, Clone)]
pub struct Signals {
    pub task: TaskType,
    pub task_confidence: f64,
    pub very_hard_p: f64,
    pub security_p: f64,
    pub edit_p: f64,
    /// Probability the turn continues the previous work (low = a new task).
    pub continues_p: f64,
    /// Probability the user is correcting the previous answer, when it was asked.
    pub correction_p: Option<f64>,
    /// Whether the request splits into separate pieces (orchestration's trigger), and the
    /// sequence veto.
    pub orchestration: Orchestration,
    pub security_keywords: Security,
}

impl From<&ProviderWant> for Signals {
    fn from(w: &ProviderWant) -> Self {
        Self {
            task: w.task,
            task_confidence: w.task_confidence,
            very_hard_p: w.very_hard_p,
            security_p: w.security_p,
            edit_p: w.edit_p,
            continues_p: w.continues_p,
            correction_p: w.correction_p,
            orchestration: w.orchestration,
            security_keywords: w.security_keywords,
        }
    }
}

/// What the router remembers between turns.
#[derive(Debug, Clone, Default)]
pub struct RoutingState {
    /// The model the last turn really ran on.
    pub current: Option<(ProviderId, String)>,
    /// Consecutive turns the current model was kept although the chain wanted a lower tier.
    held: u32,
    /// What the last answering provider has cached; leaving it costs a rebuild.
    pub warm: Option<WarmCache>,
}

impl RoutingState {
    fn current_spec(&self) -> Option<&'static ModelSpec> {
        let (provider, id) = self.current.as_ref()?;
        find(*provider, id)
    }
}

/// All inputs of one routing decision.
pub struct RouteInput<'a> {
    /// `None` when Jev was unavailable.
    pub signals: Option<&'a Signals>,
    pub prompt_words: usize,
    pub pin: &'a Pin,
    pub usable: &'a Usable,
    pub allow: &'a Allowances,
    pub overrides: &'a Overrides,
    pub context_tokens: u64,
    /// Fraction of the plan's quota used (0..1); only Claude is steered on it.
    pub plan_utilization: &'a dyn Fn(ProviderId) -> Option<f64>,
    /// Now, in ms since 1970: how much of the warm cache's window is left.
    pub now_ms: u64,
}

/// The model for this turn and why, for the `[route]` line.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub spec: &'static ModelSpec,
    pub task: TaskType,
    pub reason: String,
    pub switched_provider: bool,
    /// The model is a lower tier than the chain's normal best.
    pub dropped_tier: bool,
}

/// Why no model can be chosen; the caller tells the user instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoRoute {
    /// The privacy policy, the budget cap or a routing rule of the user's left no model.
    Refused { why: String },
    /// No usable, allowed model exists for this kind of task (Security with only
    /// DeepSeek or Grok available lands here, by design).
    NothingUsable { task: TaskType },
    /// The `/model` pin cannot be served; it is reported, never swapped for another model.
    PinUnusable { what: String },
}

impl std::fmt::Display for NoRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingUsable { task } => write!(f, "no usable model for {task} work"),
            Self::PinUnusable { what } | Self::Refused { why: what } => f.write_str(what),
        }
    }
}

impl std::error::Error for NoRoute {}

/// Where the task type came from, which decides how it is described and how much
/// weight it carries when deciding to switch provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskSource {
    Jev,
    /// The hard security rule overrode Jev's task type.
    SecurityRule,
    NoSignals,
}

/// The chosen model before it is wrapped into a [`RouteDecision`].
struct Choice {
    spec: &'static ModelSpec,
    why: String,
    dropped_tier: bool,
}

/// Why a provider switch is allowed this turn.
struct Gate {
    /// Strong enough evidence that the "keep the provider if it is as good" shortcut
    /// does not apply.
    confident: bool,
    why: &'static str,
    /// A rule (security, a likely correction) wants the switch whatever the cache holds.
    forced: bool,
}

/// Choose the model for this turn. Updates only the hold counter; call
/// [`record_turn`] once the turn has really run, because failover may differ.
pub fn decide(state: &mut RoutingState, input: &RouteInput) -> Result<RouteDecision, NoRoute> {
    let (task, source) = effective_task(input.signals);
    let very_hard = input.signals.is_some_and(|s| s.very_hard_p >= TOP_THRESHOLD);
    let chain = chain_with(task, very_hard, input.overrides);
    let choice = match input.pin {
        Pin::Auto => auto_choice(state, input, task, source, &chain)?,
        Pin::Provider(p) => {
            state.held = 0;
            pin_provider(*p, input, task, &chain)?
        }
        Pin::Model(p, id) => {
            state.held = 0;
            pin_model(*p, id, input, task)?
        }
    };
    let switched_provider = state.current.as_ref().is_some_and(|(p, _)| *p != choice.spec.provider);
    let reason = format!(
        "task: {} → {} {}; {}",
        task_label(task, source, input.signals),
        choice.spec.provider,
        choice.spec.id,
        choice.why
    );
    Ok(RouteDecision { spec: choice.spec, task, reason, switched_provider, dropped_tier: choice.dropped_tier })
}

/// Remember the model the turn really ran on. A different model starts a fresh cache,
/// so the hold counter restarts.
pub fn record_turn(state: &mut RoutingState, spec: &ModelSpec) {
    let now = (spec.provider, spec.id.to_string());
    if state.current.as_ref() != Some(&now) {
        state.held = 0;
        state.current = Some(now);
    }
}

// ---------------------------------------------------------------- task type

/// The task type that picks the chain. Security is a HARD rule that overrides Jev:
/// a confident-looking wrong answer is costly there and Jev's task label alone does
/// not reliably catch it. Without Jev there is no judgment, so the task is Unclear.
fn effective_task(signals: Option<&Signals>) -> (TaskType, TaskSource) {
    let Some(s) = signals else { return (TaskType::Unclear, TaskSource::NoSignals) };
    let security_rule = s.security_keywords == Security::Strong
        || s.security_p >= CODE_SECURITY_P
        || (s.security_keywords == Security::Weak && s.edit_p >= CODE_EDIT_P);
    if security_rule && s.task != TaskType::Security {
        (TaskType::Security, TaskSource::SecurityRule)
    } else {
        (s.task, TaskSource::Jev)
    }
}

fn task_label(task: TaskType, source: TaskSource, signals: Option<&Signals>) -> String {
    match (source, signals) {
        (TaskSource::Jev, Some(s)) => format!("{task} ({:.2})", s.task_confidence),
        (TaskSource::SecurityRule, _) => format!("{task} (security rule)"),
        _ => format!("{task} (Jev unavailable)"),
    }
}

// ---------------------------------------------------------------- usability

/// Whether `spec` may serve this turn: usable, billing-allowed, allowed for the task
/// and big enough for the session. Delegates to [`pick`] so the one definition of
/// "may serve" lives in the catalog and cannot drift from it.
fn can_serve(spec: &'static ModelSpec, task: TaskType, input: &RouteInput) -> bool {
    pick(&[spec], task, input.usable, input.allow, input.context_tokens).is_some()
}

/// Plain-words reason a model cannot serve, checked in [`pick`]'s order.
fn why_unusable(spec: &ModelSpec, task: TaskType, input: &RouteInput) -> &'static str {
    if !input.usable.contains(spec) {
        "not logged in or not entitled"
    } else if !allowed_for(task, spec) {
        "barred for this kind of work"
    } else if spec.context_tokens < input.context_tokens {
        "the session is larger than its context window"
    } else {
        "its billing (credits or API key) is not enabled"
    }
}

// ---------------------------------------------------------------- pins

/// A pinned model is used exactly or not at all.
fn pin_model(provider: ProviderId, id: &str, input: &RouteInput, task: TaskType) -> Result<Choice, NoRoute> {
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
fn pin_provider(provider: ProviderId, input: &RouteInput, task: TaskType, chain: &[&'static ModelSpec]) -> Result<Choice, NoRoute> {
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

// ---------------------------------------------------------------- automatic routing

fn auto_choice(
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

/// Apply stickiness to the chain's best pick.
fn sticky(
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
        let why = format!("keeping {}: its {} token cache is warm and no stronger model is on offer", cur.provider, crate::uifmt::human_count(tokens));
        return hold_within(state, task, cur, &own, why);
    }
    state.held = 0;
    let why = format!("switched from {} ({})", cur.provider, gate.why);
    Choice { spec: best.spec, why, dropped_tier: best.dropped_tier }
}

/// Whether the turn may change provider. A provider switch costs a cold session plus
/// a handoff, so it needs one of: a confident new task (not a short follow-up), a
/// likely correction (the current provider may be the problem), or Security work
/// (the hard rule must reach the models that are allowed for it).
fn switch_gate(input: &RouteInput, task: TaskType, source: TaskSource) -> Option<Gate> {
    let s = input.signals?;
    if task == TaskType::Security {
        return Some(Gate { confident: source == TaskSource::SecurityRule || s.task_confidence >= CONFIDENT_SWITCH, why: "security work", forced: true });
    }
    if s.correction_p.is_some_and(|p| p >= CORRECTION_P) {
        return Some(Gate { confident: true, why: "likely correction, current provider may be the problem", forced: true });
    }
    let new_task = s.continues_p < CONTINUES_P
        && input.prompt_words > SHORT_FOLLOW_UP_WORDS
        && s.task_confidence >= MIN_BOUNDARY_CONFIDENCE;
    new_task.then_some(Gate { confident: s.task_confidence >= CONFIDENT_SWITCH, why: "new task, confident", forced: false })
}

fn stay(spec: &'static ModelSpec, why: String, dropped_tier: bool) -> Choice {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};
    use crate::catalog::{self, Billing, default_chain};

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

    fn sig(task: TaskType) -> Signals {
        Signals {
            task,
            task_confidence: 0.9,
            very_hard_p: 0.0,
            security_p: 0.0,
            edit_p: 0.0,
            continues_p: 0.0,
            correction_p: None,
            orchestration: Orchestration::default(),
            security_keywords: Security::None,
        }
    }

    struct Fixture {
        usable: Usable,
        allow: Allowances,
        overrides: Overrides,
        pin: Pin,
        context_tokens: u64,
        plan: Option<f64>,
        now_ms: u64,
    }

    impl Fixture {
        fn new(providers: &[ProviderId]) -> Self {
            Self {
                usable: Usable::all_of(providers.iter().copied()),
                allow: Allowances { credits: true, per_token: true },
                overrides: Overrides::default(),
                pin: Pin::Auto,
                context_tokens: 0,
                plan: None,
                now_ms: 0,
            }
        }

        fn run(&self, state: &mut RoutingState, signals: Option<&Signals>, words: usize) -> Result<RouteDecision, NoRoute> {
            let plan = self.plan;
            let util = move |p: ProviderId| if p == Claude { plan } else { None };
            decide(
                state,
                &RouteInput {
                    signals,
                    prompt_words: words,
                    pin: &self.pin,
                    usable: &self.usable,
                    allow: &self.allow,
                    overrides: &self.overrides,
                    context_tokens: self.context_tokens,
                    plan_utilization: &util,
                    now_ms: self.now_ms,
                },
            )
        }
    }

    fn on(id: &str) -> RoutingState {
        let spec = models().iter().find(|m| m.id == id).unwrap();
        let mut s = RoutingState::default();
        record_turn(&mut s, spec);
        s
    }

    const NEW_TASK_WORDS: usize = 20;

    #[test]
    fn pin_is_honored_or_an_error_never_swapped() {
        // (pin, providers, task, expected model or None for an error)
        let cases: Vec<(Pin, Vec<ProviderId>, TaskType, Option<&str>)> = vec![
            (Pin::Model(DeepSeek, "deepseek-flash".into()), vec![Claude, DeepSeek], TaskType::HardDebug, Some("deepseek-flash")),
            (Pin::Model(DeepSeek, "deepseek-flash".into()), vec![Claude], TaskType::Explain, None),
            (Pin::Model(Claude, "nope".into()), vec![Claude], TaskType::Explain, None),
            (Pin::Provider(Codex), vec![Claude, Codex], TaskType::Feature, Some("gpt-6.1-terra")),
            (Pin::Provider(Codex), vec![Claude], TaskType::Feature, None),
            // Pinned to a vendor the Security rule bars: reported, not rerouted.
            (Pin::Provider(Grok), vec![Claude, Grok], TaskType::Security, None),
        ];
        for (pin, provs, task, want) in cases {
            let mut f = Fixture::new(&provs);
            f.pin = pin.clone();
            let got = f.run(&mut RoutingState::default(), Some(&sig(task)), NEW_TASK_WORDS);
            match want {
                Some(id) => assert_eq!(got.unwrap().spec.id, id, "{pin:?}"),
                None => assert!(matches!(got, Err(NoRoute::PinUnusable { .. })), "{pin:?}: {got:?}"),
            }
        }
    }

    #[test]
    fn pin_respects_billing_and_context() {
        let mut f = Fixture::new(&[Claude]);
        f.pin = Pin::Model(Claude, "claude-fable-5-2".into());
        f.allow.credits = false;
        let err = f.run(&mut RoutingState::default(), Some(&sig(TaskType::Feature)), 5).unwrap_err();
        assert!(err.to_string().contains("billing"), "{err}");
        f.allow.credits = true;
        assert!(f.run(&mut RoutingState::default(), None, 5).is_ok());
        let mut f = Fixture::new(&[Codex]);
        f.pin = Pin::Model(Codex, "gpt-6.1-sol".into());
        f.context_tokens = 600_000;
        assert!(matches!(f.run(&mut RoutingState::default(), None, 5), Err(NoRoute::PinUnusable { .. })));
    }

    #[test]
    fn security_rule_beats_jevs_task_type() {
        let strong = Signals { security_keywords: Security::Strong, ..sig(TaskType::Docs) };
        let by_p = Signals { security_p: 0.9, ..sig(TaskType::Docs) };
        let weak_edit = Signals { security_keywords: Security::Weak, edit_p: 0.6, ..sig(TaskType::Docs) };
        let weak_no_edit = Signals { security_keywords: Security::Weak, edit_p: 0.1, ..sig(TaskType::Docs) };
        let f = Fixture::new(&PROVIDERS);
        for (s, security) in [(strong, true), (by_p, true), (weak_edit, true), (weak_no_edit, false)] {
            let d = f.run(&mut RoutingState::default(), Some(&s), NEW_TASK_WORDS).unwrap();
            assert_eq!(d.task == TaskType::Security, security, "{s:?}");
            if security {
                assert!(d.reason.contains("security rule"), "{}", d.reason);
                assert!(allowed_for(TaskType::Security, d.spec));
            }
        }
    }

    #[test]
    fn security_with_only_deepseek_or_grok_is_nothing_usable() {
        let s = Signals { security_keywords: Security::Strong, ..sig(TaskType::Feature) };
        for provs in [vec![DeepSeek], vec![Grok], vec![DeepSeek, Grok]] {
            let f = Fixture::new(&provs);
            assert_eq!(
                f.run(&mut RoutingState::default(), Some(&s), 20).unwrap_err(),
                NoRoute::NothingUsable { task: TaskType::Security }
            );
        }
    }

    #[test]
    fn no_signals_stays_on_the_current_model_if_usable() {
        let f = Fixture::new(&PROVIDERS);
        let mut st = on("deepseek-flash");
        let d = f.run(&mut st, None, 3).unwrap();
        assert_eq!((d.spec.id, d.switched_provider, d.task), ("deepseek-flash", false, TaskType::Unclear));
        // No current model: the Unclear chain's first choice.
        let d = f.run(&mut RoutingState::default(), None, 3).unwrap();
        assert_eq!(d.spec.id, "claude-sonnet-5-5");
        // The current model is gone: move to a usable one.
        let f = Fixture::new(&[Codex]);
        let d = f.run(&mut on("deepseek-flash"), None, 3).unwrap();
        assert_eq!(d.spec.id, "gpt-6.1-terra");
        assert!(d.switched_provider);
    }

    #[test]
    fn short_follow_up_and_continuation_stay_put() {
        let f = Fixture::new(&PROVIDERS);
        // SimpleEdit's best is DeepSeek, but the session is on Claude.
        let new_task = sig(TaskType::SimpleEdit);
        let d = f.run(&mut on("claude-sonnet-5-5"), Some(&new_task), 3).unwrap();
        assert_eq!(d.spec.id, "claude-sonnet-5-5");
        assert!(d.reason.contains("warm session"), "{}", d.reason);
        let continues = Signals { continues_p: 0.9, ..sig(TaskType::SimpleEdit) };
        assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&continues), NEW_TASK_WORDS).unwrap().spec.id, "claude-sonnet-5-5");
        let vague = Signals { task_confidence: 0.4, ..sig(TaskType::SimpleEdit) };
        assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&vague), NEW_TASK_WORDS).unwrap().spec.id, "claude-sonnet-5-5");
    }

    #[test]
    fn correction_allows_a_switch() {
        let f = Fixture::new(&PROVIDERS);
        let corr = Signals { correction_p: Some(0.8), task_confidence: 0.55, ..sig(TaskType::SimpleEdit) };
        let d = f.run(&mut on("claude-sonnet-5-5"), Some(&corr), 3).unwrap();
        assert_eq!(d.spec.id, "deepseek-flash");
        assert!(d.switched_provider);
        let weak = Signals { correction_p: Some(0.3), ..corr };
        assert_eq!(f.run(&mut on("claude-sonnet-5-5"), Some(&weak), 3).unwrap().spec.id, "claude-sonnet-5-5");
    }

    #[test]
    fn unconfident_boundary_keeps_the_provider_confident_switches() {
        let f = Fixture::new(&PROVIDERS);
        let unsure = Signals { task_confidence: 0.6, ..sig(TaskType::SimpleEdit) };
        let d = f.run(&mut on("claude-sonnet-5-5"), Some(&unsure), NEW_TASK_WORDS).unwrap();
        assert_eq!((d.spec.id, d.switched_provider), ("claude-sonnet-5-5", false));
        assert!(d.reason.contains("not confident enough"), "{}", d.reason);
        let sure = Signals { task_confidence: 0.8, ..sig(TaskType::SimpleEdit) };
        let d = f.run(&mut on("claude-sonnet-5-5"), Some(&sure), NEW_TASK_WORDS).unwrap();
        assert_eq!((d.spec.id, d.switched_provider), ("deepseek-flash", true));
        assert!(d.reason.contains("switched from claude"), "{}", d.reason);
        // Unconfident, but the current provider's best is weaker than the pick: switch anyway.
        let unsure_hard = Signals { task_confidence: 0.6, ..sig(TaskType::HardDebug) };
        let d = f.run(&mut on("claude-haiku-5-5"), Some(&unsure_hard), NEW_TASK_WORDS).unwrap();
        assert_eq!(d.spec.provider, Claude); // Opus is on the chain: provider kept, upgraded
        assert_eq!(d.spec.id, "claude-opus-5-5");
    }

    #[test]
    fn a_big_warm_cache_keeps_the_provider_until_it_expires_or_a_rule_forces_the_switch() {
        let mut f = Fixture::new(&PROVIDERS);
        let sure = Signals { task_confidence: 0.8, ..sig(TaskType::SimpleEdit) };
        let warm = |tokens| {
            let mut s = on("claude-sonnet-5-5");
            s.warm = Some(WarmCache { provider: Claude, tokens, seen_ms: 0 });
            s
        };
        f.now_ms = 60_000;
        let d = f.run(&mut warm(80_000), Some(&sure), NEW_TASK_WORDS).unwrap();
        assert_eq!((d.spec.provider, d.switched_provider), (Claude, false));
        assert!(d.reason.contains("80K token cache is warm"), "{}", d.reason);
        // A small cache is cheap to rebuild.
        assert!(f.run(&mut warm(5_000), Some(&sure), NEW_TASK_WORDS).unwrap().switched_provider);
        // The window is over: nothing is left to lose.
        f.now_ms = 400_000;
        assert!(f.run(&mut warm(80_000), Some(&sure), NEW_TASK_WORDS).unwrap().switched_provider);
        // A likely correction switches whatever the cache holds.
        f.now_ms = 60_000;
        let corr = Signals { correction_p: Some(0.8), ..sure };
        assert!(f.run(&mut warm(80_000), Some(&corr), 3).unwrap().switched_provider);
    }

    #[test]
    fn plan_steering_moves_off_claude_but_not_when_it_is_the_only_option() {
        let mut f = Fixture::new(&[Claude, Codex]);
        let s = sig(TaskType::Feature);
        assert_eq!(f.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.id, "claude-sonnet-5-5");
        f.plan = Some(0.89);
        assert_eq!(f.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.provider, Claude);
        f.plan = Some(0.90);
        let d = f.run(&mut RoutingState::default(), Some(&s), 20).unwrap();
        assert_eq!(d.spec.id, "gpt-6.1-terra");
        assert!(d.reason.contains("steered off claude"), "{}", d.reason);
        // At a boundary the steered chain also stops "keep the provider" from picking Claude.
        let d = f.run(&mut on("claude-sonnet-5-5"), Some(&Signals { task_confidence: 0.6, ..s.clone() }), 20).unwrap();
        assert_eq!(d.spec.provider, Codex);
        // Claude is the only usable provider: it still serves.
        let mut only = Fixture::new(&[Claude]);
        only.plan = Some(0.99);
        assert_eq!(only.run(&mut RoutingState::default(), Some(&s), 20).unwrap().spec.id, "claude-sonnet-5-5");
    }

    #[test]
    fn hold_steps_down_after_max_hold_and_upgrade_is_immediate() {
        let f = Fixture::new(&[Claude]);
        let easy = sig(TaskType::Feature); // chain wants Sonnet
        let mut st = on("claude-opus-5-5");
        for turn in 1..MAX_HOLD {
            let d = f.run(&mut st, Some(&easy), 3).unwrap();
            assert_eq!(d.spec.id, "claude-opus-5-5", "turn {turn} should hold");
            assert!(d.reason.contains("holding"), "{}", d.reason);
            record_turn(&mut st, d.spec);
        }
        let d = f.run(&mut st, Some(&easy), 3).unwrap();
        assert_eq!(d.spec.id, "claude-sonnet-5-5");
        assert!(d.reason.contains("stepped down"), "{}", d.reason);
        record_turn(&mut st, d.spec);
        assert_eq!(st.held, 0);
        // Upgrade is immediate and resets the counter.
        let mut st = on("claude-sonnet-5-5");
        let d = f.run(&mut st, Some(&sig(TaskType::HardDebug)), 3).unwrap();
        assert_eq!(d.spec.id, "claude-opus-5-5");
        assert!(d.reason.contains("upgrade"), "{}", d.reason);
        // A hard turn in the middle of a hold restarts it.
        let mut st = on("claude-opus-5-5");
        f.run(&mut st, Some(&easy), 3).unwrap();
        f.run(&mut st, Some(&sig(TaskType::HardDebug)), 3).unwrap();
        assert_eq!(st.held, 0);
    }

    #[test]
    fn step_down_is_one_tier_and_stays_on_the_chain_and_usable() {
        let o = catalog::load_overrides(
            r#"{"simple_edit":["claude:claude-haiku-5-5","claude:claude-sonnet-5-5","claude:claude-opus-5-5"]}"#,
        )
        .unwrap();
        // Chain order puts Haiku first, but from Opus the step is to Sonnet, not Haiku.
        let mut f = Fixture::new(&[Claude]);
        f.overrides = o;
        let mut st = on("claude-opus-5-5");
        st.held = MAX_HOLD - 1;
        let d = f.run(&mut st, Some(&sig(TaskType::SimpleEdit)), 3).unwrap();
        assert_eq!(d.spec.id, "claude-sonnet-5-5");
        // Sonnet unusable: the next best servable entry is Haiku.
        f.usable = Usable::new([(Claude, "claude-haiku-5-5"), (Claude, "claude-opus-5-5")]);
        let mut st = on("claude-opus-5-5");
        st.held = MAX_HOLD - 1;
        assert_eq!(f.run(&mut st, Some(&sig(TaskType::SimpleEdit)), 3).unwrap().spec.id, "claude-haiku-5-5");
    }

    #[test]
    fn record_turn_updates_current_and_resets_hold_on_a_model_change() {
        let mut st = RoutingState::default();
        let sonnet = catalog::find(Claude, "claude-sonnet-5-5").unwrap();
        record_turn(&mut st, sonnet);
        assert_eq!(st.current, Some((Claude, "claude-sonnet-5-5".to_string())));
        st.held = 2;
        record_turn(&mut st, sonnet);
        assert_eq!(st.held, 2);
        record_turn(&mut st, catalog::find(Codex, "gpt-6.1-sol").unwrap());
        assert_eq!((st.current.clone(), st.held), (Some((Codex, "gpt-6.1-sol".to_string())), 0));
    }

    #[test]
    fn current_model_too_big_for_the_session_is_left() {
        let mut f = Fixture::new(&[Codex, Claude]);
        f.context_tokens = 600_000;
        let d = f.run(&mut on("gpt-6.1-terra"), Some(&sig(TaskType::Feature)), 3).unwrap();
        assert_eq!(d.spec.provider, Claude);
        assert!(d.switched_provider);
    }

    #[test]
    fn short_follow_up_limit_matches_the_router() {
        let words = |n: usize| vec!["w"; n].join(" ");
        for n in 0..20 {
            assert_eq!(n <= SHORT_FOLLOW_UP_WORDS, crate::router::is_short_follow_up(&words(n)), "{n}");
        }
    }

    #[test]
    fn staying_warm_reports_a_drop_only_below_what_the_task_needs() {
        // Explain, Claude warm on Sonnet, Grok 4.7 (Top) also ready: a short follow-up
        // stays on Sonnet, which is all explaining needs, so it is not "lower tier".
        let f = Fixture::new(&[Claude, Grok]);
        let mut st = RoutingState { current: Some((Claude, "claude-sonnet-5-5".into())), ..Default::default() };
        let d = f.run(&mut st, Some(&sig(TaskType::Explain)), 3).unwrap();
        assert_eq!(d.spec.id, "claude-sonnet-5-5", "{}", d.reason);
        assert!(!d.dropped_tier, "{}", d.reason);
    }

    #[test]
    fn dropped_tier_comes_from_the_pick() {
        // Sonnet is what explaining needs, even though Grok 4.7 heads the chain.
        let f = Fixture::new(&[Claude]);
        assert!(!f.run(&mut RoutingState::default(), Some(&sig(TaskType::Explain)), 20).unwrap().dropped_tier);
        // Only a light model left for an explanation: a real drop.
        let f = Fixture::new(&[Gemini]);
        assert!(f.run(&mut RoutingState::default(), Some(&sig(TaskType::Explain)), 20).unwrap().dropped_tier);
    }

    /// Tiny deterministic generator so the property test needs no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn unit(&mut self) -> f64 {
            (self.next() % 1001) as f64 / 1000.0
        }
        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.next() as usize % items.len()]
        }
    }

    #[test]
    fn a_decision_never_leaves_the_usable_set() {
        let mut rng = Rng(7);
        for mask in 0..32u32 {
            let provs: Vec<_> = PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect();
            for task in TaskType::ALL {
                for round in 0..40 {
                    let mut f = Fixture::new(&provs);
                    f.allow = Allowances { credits: rng.next() % 2 == 0, per_token: rng.next() % 2 == 0 };
                    f.plan = if rng.next() % 2 == 0 { Some(rng.unit()) } else { None };
                    f.context_tokens = rng.pick(&[0, 300_000, 700_000]);
                    f.pin = match rng.next() % 6 {
                        0 => Pin::Provider(rng.pick(&PROVIDERS)),
                        1 => {
                            let m = rng.pick(&models().iter().collect::<Vec<_>>());
                            Pin::Model(m.provider, m.id.to_string())
                        }
                        _ => Pin::Auto,
                    };
                    let s = Signals {
                        task,
                        task_confidence: rng.unit(),
                        very_hard_p: rng.unit(),
                        security_p: rng.unit(),
                        edit_p: rng.unit(),
                        continues_p: rng.unit(),
                        correction_p: if rng.next() % 2 == 0 { Some(rng.unit()) } else { None },
                        orchestration: Orchestration::default(),
                        security_keywords: rng.pick(&[Security::None, Security::Weak, Security::Strong]),
                    };
                    let signals = (round % 5 != 0).then_some(&s);
                    // Start from a random session, including a model that has since become unusable.
                    let mut st = RoutingState::default();
                    if rng.next() % 3 != 0 {
                        record_turn(&mut st, rng.pick(&models().iter().collect::<Vec<_>>()));
                    }
                    for _ in 0..3 {
                        let words = rng.pick(&[3, 9, 30]);
                        if let Ok(d) = f.run(&mut st, signals, words) {
                            assert!(provs.contains(&d.spec.provider), "mask {mask} {task} {:?}", d.spec.id);
                            assert!(f.usable.contains(d.spec));
                            assert!(allowed_for(d.task, d.spec), "{} for {}", d.spec.id, d.task);
                            assert!(d.spec.context_tokens >= f.context_tokens);
                            assert!(match d.spec.billing {
                                Billing::Subscription => true,
                                Billing::Credits => f.allow.credits,
                                Billing::PerToken => f.allow.per_token,
                            });
                            assert!(!d.reason.is_empty());
                            record_turn(&mut st, d.spec);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn auto_with_no_failure_always_routes_when_something_in_the_chain_is_usable() {
        // The non-error side of the property: a usable chain entry means an answer.
        let f = Fixture::new(&[Claude]);
        for task in TaskType::ALL {
            let chain_has_claude = default_chain(task, false).iter().any(|s| s.provider == Claude && s.billing != Billing::Credits);
            let got = f.run(&mut RoutingState::default(), Some(&sig(task)), 20);
            assert_eq!(got.is_ok(), chain_has_claude, "{task}");
        }
    }
}
