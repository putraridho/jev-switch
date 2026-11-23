//! The prefix a model has cached after its last turn, and what leaving it would cost.
//! Pure (clocks and usage are passed in). The router reads it to prefer a warm model
//! ([`worth_keeping`]); the turn engine and the `/cache` dashboard read it for switch notes
//! and countdowns.
//!
//! Everything here is advisory. What a cache holds is only what the provider reported as read
//! or written; how long it lives and what a read costs are priors until [`crate::domain::cache_facts`]
//! has counted turns, and each says which it is. A warm cache is weighed after the quality
//! floor, the hard rules, quota steering and a pin have decided what is allowed (see
//! `decide::stickiness`); it never widens that set.

use jev_shared::domain::wording::{human_count, human_duration};
use jev_shared::{ProviderId, Usage};

use super::cache_facts::{ ttl_of, Basis, CacheFacts, CacheKey, Ttl };

pub use super::cache_facts::prior_ttl_ms as ttl_ms;

/// What a cached input token costs against an uncached one, in percent. Anthropic bills a
/// cache read at 10% of the input price; the others publish similar discounts. A prior: the
/// real price of a read is not something a usage counter shows.
pub const READ_COST_PERCENT: u64 = 10;
/// A context smaller than this is cheap to rebuild: not worth holding a provider for.
pub const STICK_MIN_TOKENS: u64 = 20_000;
/// Warn about an expiring window when this little of it is left.
const WARN_BELOW_MS: u64 = 60_000;

/// A cache that read less than this share of its input (percent) over at least
/// [`MIN_HIT_SAMPLES`] turns is not worth holding a model for.
const MIN_HIT_PERCENT:u64 = 30;
const MIN_HIT_SAMPLES: u32 = 3;

/// The observed hit ratio behind a key, as it was when the cache was marked warm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRatio {
    pub percent: u64,
    pub samples: u32,
}

/// The prefix a model has cached after its last turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmCache {
    pub provider: ProviderId,
    pub model: String,
    /// The effective backend's identity: the same model elsewhere has its own cache.
    pub backend: String,
    /// Tokens the provider reported cached (read or written) at that turn; with no counters,
    /// the request size, which is then `assumed`.
    pub tokens: u64,
    pub tokens_basis: Basis,
    /// When that turn ended, in ms since 1970.
    pub seen_ms: u64,
    pub ttl: Ttl,
    /// What counted turns say about this cache, when any did.
    pub hit: Option<HitRatio> }

impl WarmCache {
    /// What the provider holds after a turn. With counters, only the part it reported as read
    /// or written is cached (never the whole context); without them the request size is a guess
    /// and says so.
    pub fn after_turn(
        key: &CacheKey, usage: &Usage, ended_ms: u64,
        facts: Option<&CacheFacts>) -> Self {
        let (tokens, tokens_basis) = if usage.counters_known() {
            let read = usage.cached_input_tokens.min(usage.input_tokens);
            (
                read
                    .saturating_add(u64::from(usage.cache_written_tokens.unwrap_or(0)))
                    .min(usage.input_tokens),
                Basis::Observed,
            )
        } else {
            ( usage.context_tokens.unwrap_or(0), Basis::Assumed)
        };
        let hit = facts.and_then(|f|
            Some(HitRatio { percent: f.hit_percent()?, samples: f.samples })
        );
        Self {
            provider: key.provider,
            model: key.model.clone(),
            backend: key.backend.clone(),
            tokens,
            tokens_basis, seen_ms: ended_ms,
            ttl: ttl_of(key.provider, facts),
            hit }
    }

    pub fn key(&self) -> CacheKey {
        CacheKey::new(self.provider, &self.model, &self.backend)
    }

    /// Whether this is the cache of `provider`'s `model`.
    pub fn is_of(&self, provider: ProviderId, model: &str) -> bool {
        self.provider == provider && self.model == model
    }

    /// Time left in the believed window.
    pub fn remaining_ms(&self, now_ms: u64) ->u64 { (self.seen_ms + self. ttl.ms).saturating_sub(now_ms)
    }

    /// Whether the prefix can still be read, as far as is believed. A provider that documents
    /// no window is believed warm only for a short while, never for ever.
    pub fn is_warm(&self, now_ms: u64) -> bool {
        self.remaining_ms(now_ms) > 0
    }

    /// Counted turns say this cache rarely hits, so holding a model for it buys little.
    pub fn unreliable(&self) -> bool {
        self.hit.is_some_and(|h| h.samples >= MIN_HIT_SAMPLES && h.percent < MIN_HIT_PERCENT)
    }
}

/// What moving the next turn to another model would cost in cache terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchEstimate {
    /// Tokens the new model reads uncached, which the warm one would have read cached.
    pub rebuild_tokens: u64,
    /// The extra input cost, in uncached-token equivalents (assuminga cached read costs
    /// [`READ_COST_PERCENT`]).
    pub extra_cost_tokens: u64,
    /// The size comes from request sizes, not reported counters.
    pub tokens_assumed: bool }

/// The cost of leaving the warm cache for `provider`'s `model`. `None` when nothing is warm or
/// that model is the one that holds it: there is nothing to lose.
pub fn switch_estimate(warm: Option<&WarmCache>,
    provider: ProviderId,
    model: &str, now_ms: u64) -> Option<SwitchEstimate> {
    let warm = warm.filter(|w| ! w.is_of(provider, model) && w.is_warm(now_ms) && w.tokens > 0)?;
    Some(SwitchEstimate {
        rebuild_tokens: warm.tokens,
        extra_cost_tokens: ( warm.tokens * (100 - READ_COST_PERCENT)) / 100,
        tokens_assumed: warm.tokens_basis == Basis::Assumed })
}

/// Whether the router should prefer staying with the warm model: the cache is believed alive,
/// big enough for a rebuild to matter, and not one that counted turns show rarely hits.
pub fn worth_keeping(warm: Option<&WarmCache>, provider: ProviderId,
    model: &str, now_ms: u64) -> bool {
    warm.is_some_and(|w| w.is_of(provider, model) && w.tokens >= STICK_MIN_TOKENS && w.is_warm(now_ms) &&
            !w.unreliable())
}

/// Footer and warning text for the countdown.
pub fn countdown(warm: &WarmCache, now_ms: u64) ->String {
    let basis = warm.ttl.basis.label();
    match warm.remaining_ms(now_ms) {
        0 =>
        format!("{} cache expired ({basis} window)", warm.provider),
        left =>
        format!("{} cache {} left ({basis} window)", warm.provider, human_duration(left))
    }
}

/// A heads-up once the window is nearly used up and enough is cached to matter.
pub fn expiry_warning(warm: &WarmCache, now_ms: u64) -> Option<String> {
    let left = warm.remaining_ms(now_ms);
    (left > 0 && left <= WARN_BELOW_MS && warm.tokens >= STICK_MIN_TOKENS).then(|| {
        format!(
            "the {} cache ({} tokens) expires in {}; the next message after that rebuilds it",
            warm.provider,
            human_count(warm.tokens),
            human_duration(left)
        )
    })
}

/// The line shown before a switch of model is accepted.
pub fn switch_note(est: &SwitchEstimate, from: &WarmCache, to: &str) -> String {
    let guess = if est.tokens_assumed {
        ", sized from the request, not a reported count"
    } else {
        ""
    };
    format!(
        "switching {} {} to {to} rebuilds ~{} tokens uncached (about {} tokens of extra input cost, assuming a cached read costs {READ_COST_PERCENT}%{guess})",
        from.provider,
        from.model,
        human_count(est.rebuild_tokens),
        human_count(est.extra_cost_tokens)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, cached: u64) -> Usage {
        Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: 1, context_tokens: None, ..Usage::default() }
    }

    fn key(provider: ProviderId) -> CacheKey {
        CacheKey::new(provider, "m", "b")
    }

    fn warm(provider: ProviderId, tokens: u64, seen_ms: u64) -> WarmCache {
        WarmCache::after_turn(&key(provider), &usage(tokens, tokens), seen_ms, None)
    }

    #[test]
    fn only_the_part_the_provider_reported_is_marked_cached() {
        let w = WarmCache::after_turn(
            &key(ProviderId::Claude),
            &usage(50_000, 40_000),
            1_000,
            None
        );
        assert_eq!(
            (w.tokens, w.tokens_basis),
            (40_000, Basis::Observed),
            "the read share, not the whole context"
        );
        let written = Usage { cache_written_tokens: Some(8_000), ..usage(50_000, 40_000) };
        assert_eq!(
            WarmCache::after_turn(&key(ProviderId::Claude), &written, 0, None).tokens,
            48_000,
            "read plus written"
        );
        let over = Usage { cache_written_tokens: Some(80_000), ..usage(50_000, 40_000) };
        assert_eq!(
            WarmCache::after_turn(&key(ProviderId::Claude), &over, 0, None).tokens,
            50_000,
            "never more than was sent"
        );
        let cold = WarmCache::after_turn(&key(ProviderId::Codex), &usage(50_000, 0), 0, None);
        assert_eq!(cold.tokens, 0, "counters say nothing was cached");
        let unreported = Usage {
            measurement: jev_shared::Measurement::Unavailable,
            context_tokens: Some(30_000),
            ..Usage::default()
        };
        let guess = WarmCache::after_turn(&key(ProviderId::Codex), &unreported, 0, None);
        assert_eq!((guess.tokens, guess.tokens_basis), (30_000, Basis::Assumed), "a guess says so");
    }

    #[test]
    fn window_counts_down_from_the_last_turn() {
        let w = WarmCache::after_turn(
            &key(ProviderId::Claude), &usage(50_000, 40_000), 1_000,
            None);
        assert_eq!(w.remaining_ms(1_000 + 240_000),60_000);
        assert!(w.is_warm(1_000 + 299_999));
        assert!(!w.is_warm(1_000 + 300_000));
        assert_eq!(countdown(&w, 1_000 + 240_000), "claude cache 1m00s left (assumed window)");
        assert!(expiry_warning(&w, 1_000 + 250_000).unwrap().contains("expires in 50s"));
        assert!(expiry_warning(&w, 1_000 + 10_000).is_none());
        assert_eq!(countdown(&w, 1_000 + 400_000), "claude cache expired (assumed window)");
    }

    #[test]
    fn a_provider_without_a_documented_window_gets_a_short_assumed_one() {
        let w = warm( ProviderId::DeepSeek, 90_000, 0);
        assert!(w.is_warm(30_000));
        assert!(!w.is_warm(10_000_000), "not warm for ever");
        assert!(countdown(&w, 5).contains("assumed"));
    }

    #[test]
    fn an_observed_gap_changes_the_window_and_the_label() {
        let mut facts = CacheFacts::default();
        facts.observe(&usage(10_000, 9_000), Some(500_000), 1);
        let w = WarmCache::after_turn(
            &key(ProviderId::Gemini),
            &usage(50_000, 40_000),
            0,
            Some(&facts)
        );
        assert!(w.is_warm(400_000) && !w.is_warm(500_000));
        assert!(countdown(&w, 0).contains("observed window"));
    }

    #[test]
    fn switching_away_from_a_warm_cache_has_a_price_staying_has_none() {
        let w = warm( ProviderId::Claude, 60_000, 0);
        let est = switch_estimate(Some(&w), ProviderId::Codex, "m", 1000).unwrap();
        assert_eq!((est.rebuild_tokens, est.extra_cost_tokens, est.tokens_assumed), (60_000, 54_000, false));
        assert!(switch_estimate(Some(&w), ProviderId::Claude, "m", 1000).is_none());
        assert!(switch_estimate(Some(&w), ProviderId::Claude, "other", 1000).is_some(),
            "another model has its own cache"
        );
        assert!(
            switch_estimate(Some(&w), ProviderId::Codex, "m", 400_000).is_none(), "expired: nothing to lose");
        assert!(switch_estimate(None, ProviderId::Codex, "m", 0).is_none());
        let note = switch_note(&est, &w, "codex m");
        assert!(
            note.contains("~60K") && note.contains("assuming a cached read costs 10%"),
            "{note}"
        );
        let unreported = Usage {
            measurement: jev_shared::Measurement::Unavailable,
            context_tokens: Some(30_000),
            ..Usage::default()
        };
        let guess = WarmCache::after_turn(&key(ProviderId::Claude), &unreported, 0, None);
        let est = switch_estimate(Some(&guess), ProviderId::Codex, "m", 1).unwrap();
        assert!(switch_note(&est, &guess, "codex m").contains("not a reported count"));
    }

    #[test]
    fn only_a_big_live_reliable_cache_of_this_model_is_worth_keeping() {
        let big = warm( ProviderId::Claude, 60_000, 0);
        let small = warm(ProviderId::Claude, 5_000, 0);
        assert!(worth_keeping(Some(&big), ProviderId::Claude, "m", 1000));
        assert!(!worth_keeping(Some(&small), ProviderId::Claude, "m", 1000));
        assert!(!worth_keeping(Some(&big), ProviderId::Codex, "m", 1000));
        assert!(!worth_keeping(Some(&big), ProviderId::Claude, "other", 1000),
            "the cache is per model"
        );
        assert!(!worth_keeping(Some(&big), ProviderId::Claude, "m", 500_000));
        let mut poor = big.clone();
        poor.hit = Some(HitRatio { percent: 10, samples: 5 });
        assert!(
            !worth_keeping(Some(&poor), ProviderId::Claude, "m", 1000),
            "counted turns say it rarely hits"
        );
        poor.hit = Some(HitRatio { percent: 10, samples: 2 });
        assert!(
            worth_keeping(Some(&poor), ProviderId::Claude, "m", 1000),
            "two turns are too few to say"
        ) }
}
