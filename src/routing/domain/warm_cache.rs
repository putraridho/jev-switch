//! The prefix each provider has cached after its last turn, and what leaving it would cost.
//! Pure (clocks and usage are passed in). The router reads it to prefer a warm provider
//! ([`worth_keeping`]); the turn engine and the `/cache` dashboard read it for switch notes
//! and countdowns. The windows are the providers' documented defaults (Anthropic's ephemeral
//! cache lasts 5 minutes and is refreshed by every hit). They are estimates: a provider can
//! evict earlier, and a provider that documents no window has none here.

use crate::shared::domain::wording::{human_count, human_duration};
use crate::shared::{ProviderId, Usage};

/// What a cached input token costs against an uncached one, in percent. Anthropic bills a
/// cache read at 10% of the input price; the others publish similar discounts.
pub const READ_COST_PERCENT: u64 = 10;
/// A context smaller than this is cheap to rebuild: not worth holding a provider for.
pub const STICK_MIN_TOKENS: u64 = 20_000;
/// Warn about an expiring window when this little of it is left.
const WARN_BELOW_MS: u64 = 60_000;

/// How long a provider keeps a prefix after its last use, when it documents one.
pub fn ttl_ms(provider: ProviderId) -> Option<u64> {
    match provider {
        ProviderId::Claude | ProviderId::Codex | ProviderId::Grok => Some(300_000),
        ProviderId::DeepSeek | ProviderId::Gemini => None,
    }
}

/// The prefix a provider has cached after its last turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WarmCache {
    pub provider: ProviderId,
    /// Tokens of the request that turn sent; all of it is cached afterwards.
    pub tokens: u64,
    /// When that turn ended, in ms since 1970.
    pub seen_ms: u64,
}

impl WarmCache {
    pub fn after_turn(provider: ProviderId, usage: &Usage, ended_ms: u64) -> Self {
        Self { provider, tokens: usage.context_tokens.unwrap_or(usage.input_tokens), seen_ms: ended_ms }
    }

    /// Time left in the window; `None` when the provider documents none.
    pub fn remaining_ms(&self, now_ms: u64) -> Option<u64> {
        ttl_ms(self.provider).map(|ttl| (self.seen_ms + ttl).saturating_sub(now_ms))
    }

    /// Whether the prefix can still be read. A provider with no documented window is
    /// treated as warm: nothing says it expired.
    pub fn is_warm(&self, now_ms: u64) -> bool {
        self.remaining_ms(now_ms).is_none_or(|left| left > 0)
    }
}

/// What moving the next turn to another provider would cost in cache terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchEstimate {
    /// Tokens the new provider reads uncached, which the warm one would have read cached.
    pub rebuild_tokens: u64,
    /// The extra input cost, in uncached-token equivalents (a cached read costs 10%).
    pub extra_cost_tokens: u64,
}

/// The cost of leaving the warm cache for `to`. `None` when nothing is warm or `to` is the
/// provider that holds it: there is nothing to lose.
pub fn switch_estimate(warm: Option<&WarmCache>, to: ProviderId, now_ms: u64) -> Option<SwitchEstimate> {
    let warm = warm.filter(|w| w.provider != to && w.is_warm(now_ms))?;
    Some(SwitchEstimate {
        rebuild_tokens: warm.tokens,
        extra_cost_tokens: warm.tokens * (100 - READ_COST_PERCENT) / 100,
    })
}

/// Whether the router should prefer staying with the warm provider: the cache is alive and
/// big enough for a rebuild to matter.
pub fn worth_keeping(warm: Option<&WarmCache>, provider: ProviderId, now_ms: u64) -> bool {
    warm.is_some_and(|w| w.provider == provider && w.tokens >= STICK_MIN_TOKENS && w.is_warm(now_ms))
}

/// Footer and warning text for the countdown; `None` when there is nothing to say.
pub fn countdown(warm: &WarmCache, now_ms: u64) -> Option<String> {
    let left = warm.remaining_ms(now_ms)?;
    Some(if left == 0 {
        format!("{} cache expired", warm.provider)
    } else {
        format!("{} cache {} left", warm.provider, human_duration(left))
    })
}

/// A heads-up once the window is nearly used up and enough is cached to matter.
pub fn expiry_warning(warm: &WarmCache, now_ms: u64) -> Option<String> {
    let left = warm.remaining_ms(now_ms)?;
    (left > 0 && left <= WARN_BELOW_MS && warm.tokens >= STICK_MIN_TOKENS).then(|| {
        format!(
            "the {} cache ({} tokens) expires in {}; the next message after that rebuilds it",
            warm.provider,
            human_count(warm.tokens),
            human_duration(left)
        )
    })
}

/// The line shown before a provider switch is accepted.
pub fn switch_note(est: &SwitchEstimate, from: ProviderId, to: ProviderId) -> String {
    format!(
        "switching {from} to {to} rebuilds ~{} tokens uncached (about {} tokens of extra input cost)",
        human_count(est.rebuild_tokens),
        human_count(est.extra_cost_tokens)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: u64) -> Usage {
        Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: 1, context_tokens: None }
    }

    #[test]
    fn window_counts_down_from_the_last_turn() {
        let w = WarmCache::after_turn(ProviderId::Claude, &usage(50_000, 40_000), 1_000);
        assert_eq!(w.remaining_ms(1_000 + 240_000), Some(60_000));
        assert!(w.is_warm(1_000 + 299_999));
        assert!(!w.is_warm(1_000 + 300_000));
        assert_eq!(countdown(&w, 1_000 + 240_000).unwrap(), "claude cache 1m00s left");
        assert!(expiry_warning(&w, 1_000 + 250_000).unwrap().contains("expires in 50s"));
        assert!(expiry_warning(&w, 1_000 + 10_000).is_none());
        assert_eq!(countdown(&w, 1_000 + 400_000).unwrap(), "claude cache expired");
    }

    #[test]
    fn a_provider_without_a_window_is_always_warm_and_has_no_countdown() {
        let w = WarmCache { provider: ProviderId::DeepSeek, tokens: 90_000, seen_ms: 0 };
        assert!(w.is_warm(10_000_000));
        assert!(countdown(&w, 5).is_none());
    }

    #[test]
    fn switching_away_from_a_warm_cache_has_a_price_staying_has_none() {
        let w = WarmCache { provider: ProviderId::Claude, tokens: 60_000, seen_ms: 0 };
        let est = switch_estimate(Some(&w), ProviderId::Codex, 1000).unwrap();
        assert_eq!((est.rebuild_tokens, est.extra_cost_tokens), (60_000, 54_000));
        assert!(switch_estimate(Some(&w), ProviderId::Claude, 1000).is_none());
        assert!(switch_estimate(Some(&w), ProviderId::Codex, 400_000).is_none(), "expired: nothing to lose");
        assert!(switch_estimate(None, ProviderId::Codex, 0).is_none());
        assert!(switch_note(&est, ProviderId::Claude, ProviderId::Codex).contains("~60K"));
    }

    #[test]
    fn only_a_big_live_cache_is_worth_keeping() {
        let big = WarmCache { provider: ProviderId::Claude, tokens: 60_000, seen_ms: 0 };
        let small = WarmCache { tokens: 5_000, ..big };
        assert!(worth_keeping(Some(&big), ProviderId::Claude, 1000));
        assert!(!worth_keeping(Some(&small), ProviderId::Claude, 1000));
        assert!(!worth_keeping(Some(&big), ProviderId::Codex, 1000));
        assert!(!worth_keeping(Some(&big), ProviderId::Claude, 500_000));
    }
}
