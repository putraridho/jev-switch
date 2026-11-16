//! Which models exist, and which of them may answer a given kind of task.
//!
//! Jev (a separate judgment service) decides what kind of task a turn is; this
//! module never judges a task. It only maps a [`TaskType`] to an ordered list of
//! models, and then filters that list by what the user can actually use right now.
//! Everything here is a pure function so the whole rule set is testable without a
//! network, a CLI or a clock.
//!
//! The core rule: a turn is NEVER routed to a model the user can't use. [`pick`]
//! enforces it, so a caller cannot forget to.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use serde_json::Value;

use crate::backend::ProviderId;

/// Rough capability and price band. Ordered so `Light < Everyday < Top`, which lets
/// [`pick`] tell when a fallback landed on a weaker model than the chain wanted.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Tier {
    Light,
    Everyday,
    Top,
}

/// How using a model is paid for. A model the user's plan does not cover must not be
/// picked silently, because it spends real money rather than a subscription quota.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Billing {
    /// Included in a flat plan (Claude Pro/Max, ChatGPT, ...).
    Subscription,
    /// Extra pay-as-you-go credits, e.g. Fable on Claude Pro (included on Max).
    Credits,
    /// An API key billed per token (DeepSeek).
    PerToken,
}

/// What Jev says a turn is about. The catalog maps each to an ordered model list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TaskType {
    Security,
    HardDebug,
    Review,
    Feature,
    Frontend,
    Docs,
    Explain,
    CurrentInfo,
    SimpleEdit,
    Unclear,
}

impl TaskType {
    /// Every task type, so tests and config validation can loop over all of them.
    pub const ALL: [TaskType; 10] = [
        Self::Security,
        Self::HardDebug,
        Self::Review,
        Self::Feature,
        Self::Frontend,
        Self::Docs,
        Self::Explain,
        Self::CurrentInfo,
        Self::SimpleEdit,
        Self::Unclear,
    ];

    /// Stable snake_case name used by Jev, logs and the `JEV_CHAINS` file.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::HardDebug => "hard_debug",
            Self::Review => "review",
            Self::Feature => "feature",
            Self::Frontend => "frontend",
            Self::Docs => "docs",
            Self::Explain => "explain",
            Self::CurrentInfo => "current_info",
            Self::SimpleEdit => "simple_edit",
            Self::Unclear => "unclear",
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|t| t.as_str() == s).ok_or_else(|| format!("unknown task type `{s}`"))
    }
}

/// One model the router knows about.
#[derive(Debug, PartialEq, Eq)]
pub struct ModelSpec {
    pub provider: ProviderId,
    pub id: &'static str,
    pub tier: Tier,
    pub billing: Billing,
    /// Context window; a session larger than this cannot be sent to the model.
    pub context_tokens: u64,
    /// Only tried when Jev flags the turn very hard, because these cost the most.
    pub very_hard_only: bool,
}

const K: u64 = 1_000;
const M: u64 = 1_000_000;

const fn spec(
    provider: ProviderId,
    id: &'static str,
    tier: Tier,
    billing: Billing,
    context_tokens: u64,
    very_hard_only: bool,
) -> ModelSpec {
    ModelSpec { provider, id, tier, billing, context_tokens, very_hard_only }
}

use Billing::{Credits, PerToken, Subscription};
use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};
use Tier::{Everyday, Light, Top};

/// Facts verified from the vendors' own pages on 2026-11-17. Ids are unique across
/// providers, which lets the chain tables below name a model by id alone.
/// DeepSeek Flash also serves Light work; it is listed once and Light-task chains
/// simply include it.
static MODELS: [ModelSpec; 16] = [
    spec(Claude, "claude-opus-5-5", Top, Subscription, M, false),
    spec(Claude, "claude-sonnet-5-5", Everyday, Subscription, M, false),
    spec(Claude, "claude-haiku-5-5", Light, Subscription, M, false),
    spec(Claude, "claude-fable-5-2", Top, Credits, M, true),
    spec(Codex, "gpt-6-astra", Top, Subscription, 500 * K, true),
    spec(Codex, "gpt-6.1-sol", Top, Subscription, 500 * K, false),
    spec(Codex, "gpt-6.1-terra", Everyday, Subscription, 500 * K, false),
    spec(Codex, "gpt-6-luna", Light, Subscription, 500 * K, false),
    spec(Gemini, "gemini-4-carbon", Top, Subscription, M, true),
    spec(Gemini, "gemini-4-argon", Top, Subscription, M, false),
    spec(Gemini, "gemini-4-hydrogen", Everyday, Subscription, M, false),
    spec(Gemini, "gemini-3.8-flash", Light, Subscription, M, false),
    spec(DeepSeek, "deepseek-v4-pro", Top, PerToken, M, false),
    spec(DeepSeek, "deepseek-flash", Everyday, PerToken, M, false),
    spec(Grok, "grok-4.7", Top, Subscription, 500 * K, false),
    spec(Grok, "grok-build-0.1", Everyday, Subscription, 256 * K, false),
];

/// Every known model.
pub fn models() -> &'static [ModelSpec] {
    &MODELS
}

/// Look a model up by its provider and id.
pub fn find(provider: ProviderId, id: &str) -> Option<&'static ModelSpec> {
    models().iter().find(|m| m.provider == provider && m.id == id)
}

/// Look a model up by id alone (ids are unique across providers). Only used for the
/// built-in chain tables, so an unknown id is a programming error caught by tests.
fn by_id(id: &str) -> &'static ModelSpec {
    models().iter().find(|m| m.id == id).unwrap_or_else(|| panic!("catalog chain names unknown model `{id}`"))
}

/// Hard rules that no chain, override or fallback may break. Security work must
/// never leave a vendor whose data handling the user has not vetted for it: no
/// DeepSeek (third-party API, per-token key) and no Grok.
pub fn allowed_for(task: TaskType, spec: &ModelSpec) -> bool {
    match task {
        TaskType::Security => !matches!(spec.provider, DeepSeek | Grok),
        _ => true,
    }
}

/// The default best-first model ids for a task. Entries flagged `very_hard_only` in
/// [`models`] are dropped by [`chain_with`] unless the turn is very hard, so the
/// Gemini Carbon entry in the Security list needs no special case here.
fn default_ids(task: TaskType) -> &'static [&'static str] {
    match task {
        TaskType::Security => &[
            "claude-fable-5-2",
            "gpt-6-astra",
            "claude-opus-5-5",
            "gpt-6.1-sol",
            "gemini-4-carbon",
            "gemini-4-argon",
        ],
        TaskType::HardDebug => &[
            "gpt-6-astra",
            "claude-fable-5-2",
            "gpt-6.1-sol",
            "claude-opus-5-5",
            "gemini-4-carbon",
            "gemini-4-argon",
            "deepseek-v4-pro",
        ],
        TaskType::Review => &["gpt-6.1-sol", "gemini-4-argon", "claude-opus-5-5", "deepseek-v4-pro"],
        TaskType::Feature | TaskType::Frontend | TaskType::Docs => {
            &["claude-sonnet-5-5", "gpt-6.1-terra", "gemini-4-hydrogen", "deepseek-v4-pro"]
        }
        TaskType::Explain => &["grok-4.7", "gemini-3.8-flash", "deepseek-flash", "claude-sonnet-5-5"],
        TaskType::CurrentInfo => &["grok-4.7", "gemini-3.8-flash", "claude-sonnet-5-5", "gpt-6.1-terra"],
        TaskType::SimpleEdit => &["deepseek-flash", "gpt-6-luna", "gemini-3.8-flash", "claude-sonnet-5-5"],
        TaskType::Unclear => &["claude-sonnet-5-5", "gpt-6.1-terra", "gemini-4-hydrogen"],
    }
}

/// Apply the two filters every chain must pass, whatever its source.
fn admit(
    specs: impl IntoIterator<Item = &'static ModelSpec>,
    task: TaskType,
    very_hard: bool,
) -> Vec<&'static ModelSpec> {
    specs.into_iter().filter(|s| (very_hard || !s.very_hard_only) && allowed_for(task, s)).collect()
}

/// Best-first models for `task`; `very_hard_only` models appear only if `very_hard`.
pub fn default_chain(task: TaskType, very_hard: bool) -> Vec<&'static ModelSpec> {
    admit(default_ids(task).iter().map(|id| by_id(id)), task, very_hard)
}

/// User-supplied chains that replace the defaults for the task types they list.
#[derive(Debug, Default)]
pub struct Overrides {
    chains: HashMap<TaskType, Vec<&'static ModelSpec>>,
    /// Things that were ignored (unknown task, provider or model id), for the caller to show.
    pub notes: Vec<String>,
}

impl Overrides {
    /// Read the file named by `JEV_CHAINS`, if set. A missing or broken file yields
    /// the defaults plus a note, so a typo never stops routing.
    pub fn from_env() -> Self {
        let Some(path) = std::env::var_os("JEV_CHAINS").filter(|p| !p.is_empty()) else {
            return Self::default();
        };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| load_overrides(&t)) {
            Ok(o) => o,
            Err(e) => Self { notes: vec![format!("JEV_CHAINS ignored: {e}")], ..Self::default() },
        }
    }

    /// The override for `task`, if the user listed one.
    pub fn get(&self, task: TaskType) -> Option<&[&'static ModelSpec]> {
        self.chains.get(&task).map(Vec::as_slice)
    }
}

/// Parse `{ "feature": ["claude:claude-sonnet-5-5", ...], ... }`. Pure: the caller
/// reads the file. Unknown task types, providers and model ids are skipped and
/// reported in [`Overrides::notes`]; only text that is not a JSON object is an error.
pub fn load_overrides(json: &str) -> Result<Overrides, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let obj = value.as_object().ok_or("expected a JSON object of task type to list")?;
    let mut out = Overrides::default();
    for (key, list) in obj {
        let Ok(task) = key.parse::<TaskType>() else {
            out.notes.push(format!("unknown task type `{key}` ignored"));
            continue;
        };
        let Some(items) = list.as_array() else {
            out.notes.push(format!("`{key}` must be a list; ignored"));
            continue;
        };
        let mut chain = Vec::new();
        for item in items {
            match item.as_str().ok_or_else(|| format!("{item} is not a string")).and_then(parse_model_ref) {
                Ok(spec) => chain.push(spec),
                Err(e) => out.notes.push(format!("`{key}`: {e}; ignored")),
            }
        }
        out.chains.insert(task, chain);
    }
    Ok(out)
}

/// `"<provider>:<model id>"` to a catalog entry.
fn parse_model_ref(s: &str) -> Result<&'static ModelSpec, String> {
    let (provider, id) = s.split_once(':').ok_or_else(|| format!("`{s}` is not <provider>:<model id>"))?;
    let provider: ProviderId = provider.parse()?;
    find(provider, id.trim()).ok_or_else(|| format!("unknown model `{s}`"))
}

/// Like [`default_chain`], but an override replaces the default list. The hard
/// filters still apply to overrides, so a config file cannot send Security to DeepSeek.
pub fn chain_with(task: TaskType, very_hard: bool, overrides: &Overrides) -> Vec<&'static ModelSpec> {
    match overrides.get(task) {
        Some(list) => admit(list.iter().copied(), task, very_hard),
        None => default_chain(task, very_hard),
    }
}

/// The (provider, model id) pairs the user can use. Built by the caller from
/// logins, keys and entitlements; this module does not discover them.
#[derive(Debug, Default, Clone)]
pub struct Usable {
    set: HashSet<(ProviderId, String)>,
}

impl Usable {
    pub fn new<S: AsRef<str>>(iter: impl IntoIterator<Item = (ProviderId, S)>) -> Self {
        Self { set: iter.into_iter().map(|(p, id)| (p, id.as_ref().to_string())).collect() }
    }

    /// Every catalog model of the given providers, for callers that know only
    /// which providers are logged in.
    pub fn all_of(providers: impl IntoIterator<Item = ProviderId>) -> Self {
        let providers: Vec<_> = providers.into_iter().collect();
        Self::new(models().iter().filter(|m| providers.contains(&m.provider)).map(|m| (m.provider, m.id)))
    }

    pub fn contains(&self, spec: &ModelSpec) -> bool {
        self.set.contains(&(spec.provider, spec.id.to_string()))
    }
}

/// Which spending the user has agreed to. Subscription models need no permission.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Allowances {
    /// Allow [`Billing::Credits`] models.
    pub credits: bool,
    /// Allow [`Billing::PerToken`] models; the caller sets this only if a key is configured.
    pub per_token: bool,
}

impl Allowances {
    fn permits(&self, billing: Billing) -> bool {
        match billing {
            Billing::Subscription => true,
            Billing::Credits => self.credits,
            Billing::PerToken => self.per_token,
        }
    }
}

/// The chosen model and how far down the chain it was.
#[derive(Debug, Clone, Copy)]
pub struct Pick {
    pub spec: &'static ModelSpec,
    /// Index in the chain; 0 means the first choice was available.
    pub rank: usize,
    /// The pick is a lower tier than the best the chain normally offers.
    pub dropped_tier: bool,
}

/// First chain entry that is usable, billing-allowed, allowed for the task and big
/// enough for the current session; `None` if nothing qualifies. The caller turns
/// `None` into an honest "no usable model" answer rather than guessing.
pub fn pick(
    chain: &[&'static ModelSpec],
    task: TaskType,
    usable: &Usable,
    allow: &Allowances,
    context_tokens: u64,
) -> Option<Pick> {
    let (rank, spec) = chain.iter().copied().enumerate().find(|(_, s)| {
        usable.contains(s) && allow.permits(s.billing) && allowed_for(task, s) && s.context_tokens >= context_tokens
    })?;
    // Very-hard-only entries are not the chain's normal best, so they don't count
    // as "dropping" when a very-hard turn falls from them to an everyday model.
    let best = chain.iter().filter(|s| !s.very_hard_only).map(|s| s.tier).max();
    Some(Pick { spec, rank, dropped_tier: best.is_some_and(|b| spec.tier < b) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

    fn providers_in(mask: u32) -> Vec<ProviderId> {
        PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect()
    }

    fn open() -> Allowances {
        Allowances { credits: true, per_token: true }
    }

    #[test]
    fn model_ids_are_unique_and_chains_resolve() {
        let mut ids: Vec<_> = models().iter().map(|m| m.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), models().len());
        for t in TaskType::ALL {
            for vh in [false, true] {
                assert!(!default_chain(t, vh).is_empty(), "{t} very_hard={vh}");
            }
        }
    }

    #[test]
    fn task_type_round_trips() {
        for t in TaskType::ALL {
            assert_eq!(t.as_str().parse::<TaskType>().unwrap(), t);
        }
        assert!("nope".parse::<TaskType>().is_err());
    }

    #[test]
    fn very_hard_only_models_need_very_hard() {
        for t in TaskType::ALL {
            assert!(default_chain(t, false).iter().all(|s| !s.very_hard_only));
        }
        let hard = default_chain(TaskType::HardDebug, true);
        assert_eq!(hard[0].id, "gpt-6-astra");
        assert!(default_chain(TaskType::HardDebug, false).iter().all(|s| s.id != "gpt-6-astra"));
    }

    #[test]
    fn pick_only_ever_returns_a_usable_provider() {
        for mask in 0..32u32 {
            let provs = providers_in(mask);
            let usable = Usable::all_of(provs.clone());
            for t in TaskType::ALL {
                for vh in [false, true] {
                    let chain = default_chain(t, vh);
                    if let Some(p) = pick(&chain, t, &usable, &open(), 0) {
                        assert!(provs.contains(&p.spec.provider), "{t} mask {mask}");
                        assert!(allowed_for(t, p.spec));
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_usable_gives_none() {
        let usable = Usable::all_of([]);
        for t in TaskType::ALL {
            assert!(pick(&default_chain(t, true), t, &usable, &open(), 0).is_none());
        }
    }

    #[test]
    fn allowed_for_blocks_security_on_deepseek_and_grok_for_every_task() {
        for spec in models() {
            for t in TaskType::ALL {
                let blocked = t == TaskType::Security && matches!(spec.provider, DeepSeek | Grok);
                assert_eq!(allowed_for(t, spec), !blocked, "{t} {}", spec.id);
            }
        }
        for t in TaskType::ALL {
            for vh in [false, true] {
                for s in default_chain(t, vh) {
                    assert!(allowed_for(t, s));
                }
            }
        }
    }

    #[test]
    fn security_never_picks_deepseek_or_grok() {
        let usable = Usable::all_of([DeepSeek, Grok]);
        for vh in [false, true] {
            let chain = default_chain(TaskType::Security, vh);
            assert!(pick(&chain, TaskType::Security, &usable, &open(), 0).is_none());
        }
        // Even a hand-built chain containing them cannot get through `pick`.
        let sneaky = [by_id("deepseek-v4-pro"), by_id("grok-4.7")];
        assert!(pick(&sneaky, TaskType::Security, &usable, &open(), 0).is_none());
    }

    #[test]
    fn overrides_cannot_route_security_to_deepseek() {
        let o = load_overrides(r#"{"security":["deepseek:deepseek-v4-pro","grok:grok-4.7","claude:claude-opus-5-5"]}"#)
            .unwrap();
        let chain = chain_with(TaskType::Security, true, &o);
        assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-opus-5-5"]);
    }

    #[test]
    fn credits_models_are_skipped_unless_allowed() {
        let usable = Usable::all_of([Claude]);
        let chain = default_chain(TaskType::Security, true);
        let no = Allowances { credits: false, per_token: true };
        assert_eq!(pick(&chain, TaskType::Security, &usable, &no, 0).unwrap().spec.id, "claude-opus-5-5");
        let yes = Allowances { credits: true, per_token: false };
        assert_eq!(pick(&chain, TaskType::Security, &usable, &yes, 0).unwrap().spec.id, "claude-fable-5-2");
    }

    #[test]
    fn per_token_models_are_skipped_unless_allowed() {
        let usable = Usable::all_of([DeepSeek]);
        let chain = default_chain(TaskType::SimpleEdit, false);
        let no = Allowances { credits: true, per_token: false };
        assert!(pick(&chain, TaskType::SimpleEdit, &usable, &no, 0).is_none());
        let yes = Allowances { credits: false, per_token: true };
        assert_eq!(pick(&chain, TaskType::SimpleEdit, &usable, &yes, 0).unwrap().spec.id, "deepseek-flash");
    }

    #[test]
    fn a_session_bigger_than_the_window_skips_the_model() {
        let usable = Usable::all_of([Codex, Claude]);
        let chain = default_chain(TaskType::Review, false);
        assert_eq!(pick(&chain, TaskType::Review, &usable, &open(), 400_000).unwrap().spec.id, "gpt-6.1-sol");
        let p = pick(&chain, TaskType::Review, &usable, &open(), 600_000).unwrap();
        assert_eq!(p.spec.id, "claude-opus-5-5");
        assert_eq!(p.rank, 2);
        assert!(pick(&chain, TaskType::Review, &usable, &open(), 2_000_000).is_none());
    }

    #[test]
    fn only_claude_hard_debug_picks_opus() {
        let usable = Usable::all_of([Claude]);
        let no_credits = Allowances { credits: false, per_token: false };
        let chain = default_chain(TaskType::HardDebug, false);
        let p = pick(&chain, TaskType::HardDebug, &usable, &no_credits, 0).unwrap();
        assert_eq!(p.spec.id, "claude-opus-5-5");
        assert!(!p.dropped_tier);
        // Very hard with credits on: Fable (Top) is the first usable entry.
        let chain = default_chain(TaskType::HardDebug, true);
        let p = pick(&chain, TaskType::HardDebug, &usable, &open(), 0).unwrap();
        assert_eq!(p.spec.id, "claude-fable-5-2");
    }

    #[test]
    fn dropped_tier_is_reported() {
        let chain = default_chain(TaskType::Explain, false);
        let p = pick(&chain, TaskType::Explain, &Usable::all_of([Claude]), &open(), 0).unwrap();
        assert_eq!(p.spec.id, "claude-sonnet-5-5");
        assert!(p.dropped_tier); // the chain's best is Grok (Top)
        let p = pick(&chain, TaskType::Explain, &Usable::all_of([Grok]), &open(), 0).unwrap();
        assert!(!p.dropped_tier);
    }

    #[test]
    fn overrides_replace_defaults_and_report_unknowns() {
        let o = load_overrides(
            r#"{"feature":["claude:claude-haiku-5-5","claude:nope","bogus:x","grok-4.7"],"weird":["claude:claude-opus-5-5"],"docs":"x"}"#,
        )
        .unwrap();
        assert_eq!(o.notes.len(), 5, "{:?}", o.notes);
        let chain = chain_with(TaskType::Feature, false, &o);
        assert_eq!(chain.iter().map(|s| s.id).collect::<Vec<_>>(), ["claude-haiku-5-5"]);
        // Unlisted task types keep their defaults.
        assert_eq!(chain_with(TaskType::Review, false, &o), default_chain(TaskType::Review, false));
        assert!(load_overrides("[1]").is_err());
        assert!(load_overrides("not json").is_err());
    }
}
