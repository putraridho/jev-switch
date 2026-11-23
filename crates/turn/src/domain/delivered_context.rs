//! What a request will really hold when it leaves, in the receiving provider's own tokens.
//!
//! The session's size is counted in characters. A provider turns characters into tokens at its
//! own rate, so the rate is kept per provider: a built-in prior (flagged as assumed) until a turn
//! with a reported per-request context size shows the real one. Only such a turn teaches a ratio;
//! a turn whose consumption was not reported, or that reported only a cumulative total, teaches
//! nothing, so a guess is never fed back as a measurement. Pure: sizes are passed in.

use std::collections::HashMap;

use jev_shared::{ ProviderId, Usage };

/// Characters per token before anything is measured, in thousandths (a common rule of thumb).
pub const PRIOR_MILLI: u64 = 4_000;
/// Tokens the CLI's own tool definitions and instructions add to every request, assumed.
pub const TOOLS_OVERHEAD_TOKENS: u64 = 8_000;
/// Room left for the answer, assumed.
pub const OUTPUT_HEADROOM_TOKENS: u64 = 16_000;
/// The text a catch-up adds around the missed turns (header, git status, markers), which the
/// session's own count does not hold.
pub const HANDOFF_TOKENS: u64 = 2_000;
/// A turn teaches a ratio only when this many characters were in the request: below it the
/// CLI's own fixed instructions dominate the count and would skew the rate.
const MIN_SAMPLE_CHARS: usize = 20_000;
/// A measured rate outside this range (thousandths of a character per token) is a misreading.
const MIN_MILLI: u64 = 1_500;
const MAX_MILLI: u64 = 8_000;
/// Older samples count for less once this many are in.
const SAMPLE_CAP: u32 = 20;

/// One provider's rate of characters per token and how it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharsPerToken {
    /// Thousandths of a character per token (4_000 is four characters).
    pub milli: u64,
    /// Turns it was measured from; 0 means the built-in prior, which is assumed.
    pub samples: u32,
}

impl CharsPerToken {
    pub const ASSUMED: Self = Self { milli: PRIOR_MILLI, samples: 0 };

    pub fn is_assumed(&self) -> bool {
        self.samples == 0
    }

    /// Tokens `chars` characters come to at this rate, rounded up (an estimate that guards a
    /// limit errs on the large side).
    pub fn tokens(&self, chars: usize) -> u64 {
        (chars as u64).saturating_mul(1_000).div_ceil(self.milli.max(1))
    }
}

/// The rate of every provider that has one measured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ratios(HashMap<ProviderId, CharsPerToken>);

impl Ratios {
    pub fn get(&self, provider: ProviderId) -> CharsPerToken {
        self.0.get(&provider).copied().unwrap_or(CharsPerToken::ASSUMED)
    }

    /// Learn from a finished turn: `chars` characters were in its last request, and `usage`
    /// says how many tokens that was. Returns whether the turn taught anything.
    pub fn observe(&mut self, provider: ProviderId, chars: usize, usage: &Usage) -> bool {
        let Some(tokens) = usage.context_tokens.filter(
            |t| *t > 0 && usage.measurement == jev_shared::Measurement::Reported
        ) else {
            return false;
        };
        if chars < MIN_SAMPLE_CHARS {
            return false;
        }
        let milli = (chars as u64).saturating_mul(1_000) / tokens;
        if !(MIN_MILLI..=MAX_MILLI).contains(&milli) {
            return false;
        }
        let current = self.0.get(&provider).copied().unwrap_or(CharsPerToken { milli, samples: 0 });
        let n = u64::from(current.samples.min(SAMPLE_CAP));
        let mixed = (current.milli * n + milli) / (n + 1);
        self.0.insert(provider, CharsPerToken {
            milli: mixed,
            samples: current.samples.saturating_add(1),
        });
        true
    }
}

/// What one request holds beyond the shared session size, for one provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing {
    /// Characters of the session so far.
    pub session_chars: usize,
    /// The session's size as the routing filter counts it (reported, else the rough estimate).
    pub shared_tokens: u64,
    /// Characters of the new prompt.
    pub prompt_chars: usize,
    /// The provider is behind and is caught up in the request.
    pub catch_up: bool,
}

impl Outgoing {
    /// The tokens this request adds on top of `shared_tokens` for a provider of rate `rate`:
    /// what its own tokenizer makes of the session beyond the shared figure, the new prompt,
    /// the handoff text when it is behind, the tool definitions and room for the answer.
    pub fn reserve(&self, rate: CharsPerToken) -> u64 {
        let own_session = rate.tokens(self.session_chars);
        own_session.saturating_sub(self.shared_tokens) +
            rate.tokens(self.prompt_chars) +
            (if self.catch_up { HANDOFF_TOKENS } else { 0 }) +
            TOOLS_OVERHEAD_TOKENS +
            OUTPUT_HEADROOM_TOKENS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::Measurement;
    use ProviderId::{ Claude, Codex };

    fn usage(context: Option<u64>) -> Usage {
        Usage { input_tokens: 9_999_999, context_tokens: context, ..Usage::default() }
    }

    #[test]
    fn nothing_measured_is_an_assumed_four_characters() {
        let r = Ratios::default().get(Claude);
        assert!(r.is_assumed());
        assert_eq!(r.tokens(4_000), 1_000);
        assert_eq!(r.tokens(4_001), 1_001, "rounded up");
    }

    #[test]
    fn a_turn_with_a_reported_request_size_teaches_that_providers_rate_only() {
        let mut ratios = Ratios::default();
        assert!(ratios.observe(Claude, 90_000, &usage(Some(30_000))));
        assert_eq!(ratios.get(Claude), CharsPerToken { milli: 3_000, samples: 1 });
        assert!(ratios.get(Codex).is_assumed());
        assert!(ratios.observe(Claude, 120_000, &usage(Some(30_000))));
        assert_eq!(ratios.get(Claude).milli, 3_500, "the mean of both turns");
        assert_eq!(ratios.get(Claude).samples, 2);
    }

    #[test]
    fn a_turn_without_known_usage_teaches_nothing() {
        let mut ratios = Ratios::default();
        // Cumulative consumption only: the per-request size is unknown, however big the total.
        assert!(!ratios.observe(Claude, 90_000, &usage(None)));
        let unreported = Usage {
            measurement: Measurement::Unavailable,
            context_tokens: Some(30_000),
            ..Usage::default()
        };
        assert!(!ratios.observe(Claude, 90_000, &unreported));
        assert!(!ratios.observe(Claude, 90_000, &usage(Some(0))));
        assert!(ratios.get(Claude).is_assumed());
    }

    #[test]
    fn small_sessions_and_absurd_ratios_teach_nothing() {
        let mut ratios = Ratios::default();
        assert!(
            !ratios.observe(Claude, 1_000, &usage(Some(500))),
            "too little text: the CLI's own instructions dominate"
        );
        assert!(
            !ratios.observe(Claude, 100_000, &usage(Some(90_000))),
            "1.1 characters per token is a misreading"
        );
        assert!(
            !ratios.observe(Claude, 100_000, &usage(Some(1_000))),
            "100 characters per token is a misreading"
        );
        assert!(ratios.get(Claude).is_assumed());
    }

    #[test]
    fn the_reserve_counts_the_prompt_handoff_tools_and_answer_room() {
        let out = Outgoing {
            session_chars: 400_000,
            shared_tokens: 100_000,
            prompt_chars: 4_000,
            catch_up: false,
        };
        let fixed = TOOLS_OVERHEAD_TOKENS + OUTPUT_HEADROOM_TOKENS;
        assert_eq!(out.reserve(CharsPerToken::ASSUMED), 1_000 + fixed);
        let behind = Outgoing { catch_up: true, ..out };
        assert_eq!(behind.reserve(CharsPerToken::ASSUMED), 1_000 + HANDOFF_TOKENS + fixed);
        // A provider whose tokens are smaller than four characters makes the same session bigger.
        let dense = CharsPerToken { milli: 2_000, samples: 3 };
        assert_eq!(out.reserve(dense), 100_000 + 2_000 + fixed);
    }
}
