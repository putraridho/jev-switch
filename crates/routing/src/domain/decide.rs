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
//! the work really changed (see `switch_gate`); within a provider an upgrade is
//! immediate (quality) but a downgrade is delayed (cache), see `stickiness`.
//!
//! The rules live in the child modules (`task`, `pinned`, `auto`, `switch_gate`,
//! `stickiness`); this file keeps the inputs, the outputs and the entry points.

use self::auto::auto_choice;
use self::pinned::{pin_model, pin_provider};
use self::task::{effective_task, task_label};
use jev_catalog::domain::model_table::{
    Allowances, MODEL_CHEAP, MODEL_MID, MODEL_STRONG, ModelSpec, Overrides, TaskType, Usable, chain_with, find, models,
};
use jev_judgment::domain::orchestration::Orchestration;
use jev_judgment::domain::security::Security;
use jev_judgment::domain::thresholds::TOP_THRESHOLD;
use jev_judgment::domain::want::ProviderWant;
use jev_shared::ProviderId;
use crate::domain::warm_cache::WarmCache;

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
#[derive(Clone, Copy)]
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

/// The chosen model before it is wrapped into a [`RouteDecision`].
struct Choice {
    spec: &'static ModelSpec,
    why: String,
    dropped_tier: bool,
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

mod auto;
mod pinned;
mod stickiness;
mod switch_gate;
mod task;
#[cfg(test)]
mod tests;
