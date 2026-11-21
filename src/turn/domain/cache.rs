//! Prompt-cache facts for the session: how warm each provider's cache is, what a switch
//! would rebuild, and why a turn missed the cache. Pure (clocks and usage are passed in), so
//! every rule is a unit test. [`crate::gateway::domain::prefix_fingerprint`] is the gateway-side counterpart: it sees
//! request bodies and names the byte that broke the prefix; jev-code only sees the token
//! counts each CLI reports, so causes here are the ones token counts and timing can show.
//!
//! The windows below are the providers' documented defaults (Anthropic's ephemeral cache
//! lasts 5 minutes and is refreshed by every hit). They are estimates: a provider can evict
//! earlier, and a provider that documents no window has none here.

use crate::routing::domain::warm_cache::{ttl_ms, READ_COST_PERCENT};
use crate::shared::domain::wording::human_count;
use crate::session::domain::session_log::{SessionLog, TurnRecord};

/// A turn whose cache hit rate is below this (percent) counts as a miss.
const MISS_BELOW_PERCENT: u64 = 50;

// ------------------------------------------------------------------- per turn

/// Why a turn read little from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissCause {
    /// Nothing was cached yet.
    FirstTurn,
    ProviderSwitch,
    ModelSwitch,
    /// The gap since the last turn was longer than the provider's window.
    WindowExpired,
    /// Same model, inside the window, yet the prefix was not read: something early changed
    /// (compaction, an edited instruction file, rewound history).
    PrefixChanged,
}

impl MissCause {
    pub fn label(self) -> &'static str {
        match self {
            Self::FirstTurn => "first turn",
            Self::ProviderSwitch => "switched provider",
            Self::ModelSwitch => "switched model",
            Self::WindowExpired => "window expired",
            Self::PrefixChanged => "prefix changed",
        }
    }
}

/// One turn's cache behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCache {
    pub turn: usize,
    pub model: String,
    pub read: u64,
    pub rebuilt: u64,
    /// `Some` when the turn missed the cache.
    pub miss: Option<MissCause>,
}

impl TurnCache {
    pub fn hit_percent(&self) -> u64 {
        percent(self.read, self.read + self.rebuilt)
    }
}

fn percent(part: u64, whole: u64) -> u64 {
    (part * 100).checked_div(whole).unwrap_or(0)
}

fn cause(prev: Option<&TurnRecord>, t: &TurnRecord) -> MissCause {
    let Some(prev) = prev else { return MissCause::FirstTurn };
    let (Some((pp, pm)), Some((tp, tm))) = (&prev.route, &t.route) else { return MissCause::PrefixChanged };
    if pp != tp {
        return MissCause::ProviderSwitch;
    }
    if pm != tm {
        return MissCause::ModelSwitch;
    }
    // The previous turn ended at `ended_ms`; this one started `elapsed_ms` before its own end.
    let gap = t.ended_ms.saturating_sub(t.elapsed_ms).saturating_sub(prev.ended_ms);
    match ttl_ms(*tp) {
        Some(ttl) if gap > ttl => MissCause::WindowExpired,
        _ => MissCause::PrefixChanged,
    }
}

/// Each turn that reported usage, with the cause when it missed.
pub fn per_turn(log: &SessionLog) -> Vec<TurnCache> {
    let mut out = Vec::new();
    let mut prev: Option<&TurnRecord> = None;
    for (i, t) in log.turns().iter().enumerate() {
        // A turn whose provider reported no input tokens (Antigravity over ACP) has no cache
        // data: it is not a miss.
        let Some(u) = t.usage.filter(|u| t.route.is_some() && u.input_tokens > 0) else { continue };
        let read = u.cached_input_tokens.min(u.input_tokens);
        let rebuilt = u.input_tokens - read;
        let miss = (percent(read, u.input_tokens) < MISS_BELOW_PERCENT).then(|| cause(prev, t));
        out.push(TurnCache { turn: i + 1, model: t.model.clone(), read, rebuilt, miss });
        prev = Some(t);
    }
    out
}

// ---------------------------------------------------------------- per provider

/// One provider's cache accounting over the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCache {
    pub provider: String,
    pub turns: usize,
    pub read: u64,
    pub rebuilt: u64,
}

impl ProviderCache {
    pub fn hit_percent(&self) -> u64 {
        percent(self.read, self.read + self.rebuilt)
    }

    /// What the cache saved: reads billed at 10% instead of in full, in uncached-token equivalents.
    pub fn saved_tokens(&self) -> u64 {
        self.read * (100 - READ_COST_PERCENT) / 100
    }
}

/// Cache reads and rebuilds per provider, in the order each was first used.
pub fn by_provider(log: &SessionLog) -> Vec<ProviderCache> {
    let mut rows: Vec<ProviderCache> = Vec::new();
    for t in log.turns() {
        let (Some((provider, _)), Some(u)) = (&t.route, t.usage.as_ref().filter(|u| u.input_tokens > 0)) else { continue };
        let name = provider.to_string();
        let at = rows.iter().position(|r| r.provider == name).unwrap_or_else(|| {
            rows.push(ProviderCache { provider: name, ..ProviderCache::default() });
            rows.len() - 1
        });
        let read = u.cached_input_tokens.min(u.input_tokens);
        rows[at].turns += 1;
        rows[at].read += read;
        rows[at].rebuilt += u.input_tokens - read;
    }
    rows
}

/// The one-line cache summary per provider that `/cost` appends.
pub fn cost_lines(log: &SessionLog) -> Vec<String> {
    by_provider(log)
        .iter()
        .map(|r| format!("{:<10}  hit {:>3}%  read {:>6}  rebuilt {:>6}", r.provider, r.hit_percent(), human_count(r.read), human_count(r.rebuilt)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ProviderId, Usage};
    use crate::session::domain::turn_status::TurnStatus;

    fn usage(input: u64, cached: u64) -> Usage {
        Usage { input_tokens: input, cached_input_tokens: cached, output_tokens: 1, context_tokens: None }
    }

    fn rec(provider: ProviderId, model: &str, input: u64, cached: u64, ended_ms: u64) -> TurnRecord {
        TurnRecord {
            prompt: String::new(),
            answer: "a".into(),
            status: TurnStatus::Answered,
            model: format!("{provider} · {model}"),
            route: Some((provider, model.into())),
            usage: Some(usage(input, cached)),
            elapsed_ms: 1000,
            ended_ms,
            tool_calls: 0,
        }
    }

    fn log_of(turns: Vec<TurnRecord>) -> SessionLog {
        let mut log = SessionLog::default();
        turns.into_iter().for_each(|t| log.push(t));
        log
    }

    #[test]
    fn turns_without_token_counts_are_not_cache_misses() {
        let log = log_of(vec![rec(ProviderId::Gemini, "flash", 0, 0, 1_000), rec(ProviderId::Claude, "opus", 1000, 900, 2_000)]);
        let turns = per_turn(&log);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn, 2);
        let rows = by_provider(&log);
        assert_eq!(rows.iter().map(|r| r.provider.as_str()).collect::<Vec<_>>(), ["claude"]);
    }

    #[test]
    fn misses_are_named_by_what_changed() {
        let log = log_of(vec![
            rec(ProviderId::Claude, "sonnet", 1000, 0, 2_000),
            rec(ProviderId::Claude, "sonnet", 2000, 1900, 4_000),
            rec(ProviderId::Claude, "opus", 3000, 100, 6_000),
            rec(ProviderId::Codex, "gpt", 4000, 0, 8_000),
            rec(ProviderId::Codex, "gpt", 5000, 4000, 9_000),
            // Eleven minutes of silence, then a miss on the same model.
            rec(ProviderId::Codex, "gpt", 6000, 0, 9_000 + 660_000 + 1000),
            rec(ProviderId::Codex, "gpt", 7000, 100, 700_000 + 2000),
        ]);
        let turns = per_turn(&log);
        let causes: Vec<_> = turns.iter().map(|t| t.miss).collect();
        assert_eq!(
            causes,
            vec![
                Some(MissCause::FirstTurn),
                None,
                Some(MissCause::ModelSwitch),
                Some(MissCause::ProviderSwitch),
                None,
                Some(MissCause::WindowExpired),
                Some(MissCause::PrefixChanged),
            ]
        );
        assert_eq!(turns[1].hit_percent(), 95);
    }

    #[test]
    fn per_provider_totals_and_savings() {
        let log = log_of(vec![
            rec(ProviderId::Claude, "sonnet", 1000, 0, 1),
            rec(ProviderId::Claude, "sonnet", 3000, 2000, 2),
            rec(ProviderId::Codex, "gpt", 500, 0, 3),
        ]);
        let rows = by_provider(&log);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].read, rows[0].rebuilt, rows[0].hit_percent(), rows[0].saved_tokens()), (2000, 2000, 50, 1800));
        assert_eq!(rows[1].hit_percent(), 0);
        let lines = cost_lines(&log);
        assert!(lines[0].starts_with("claude") && lines[0].contains("hit  50%"), "{lines:?}");
    }
}
