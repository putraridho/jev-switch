use super::{Billing, ModelSpec, TaskType, Tier};
use crate::shared::ProviderId;

/// Cheap model for confidently-simple turns.
pub const MODEL_CHEAP: &str = "claude-haiku-5-5";
/// Mid model for everyday coding, and for anything simple-but-uncertain or unclear.
pub const MODEL_MID: &str = "claude-sonnet-5-5";
/// Strong model for planning, tradeoffs, hard debugging, and security-sensitive work.
pub const MODEL_STRONG: &str = "claude-opus-5-5";
/// Top model for the hardest turns, used by the gateway only (`Router::with_top_threshold`).
pub const MODEL_TOP: &str = "claude-fable-5-2";

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
pub(super) fn by_id(id: &str) -> &'static ModelSpec {
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
