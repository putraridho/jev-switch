//! Prompt-cache facts for the session: how warm each provider's cache is, what a switch
//! would rebuild, and why a turn missed the cache. Pure (clocks and usage are passed in), so
//! every rule is a unit test. [`crate::prefix`] is the gateway-side counterpart: it sees
//! request bodies and names the byte that broke the prefix; jev-code only sees the token
//! counts each CLI reports, so causes here are the ones token counts and timing can show.
//!
//! The windows below are the providers' documented defaults (Anthropic's ephemeral cache
//! lasts 5 minutes and is refreshed by every hit). They are estimates: a provider can evict
//! earlier, and a provider that documents no window has none here.

use crate::backend::{ProviderId, Usage};
use crate::sessionlog::{SessionLog, TurnRecord};
use crate::shared::wording::{human_count, human_duration};
use crate::uifmt::Style;

/// What a cached input token costs against an uncached one, in percent. Anthropic bills a
/// cache read at 10% of the input price; the others publish similar discounts.
const READ_COST_PERCENT: u64 = 10;
/// A context smaller than this is cheap to rebuild: not worth holding a provider for.
pub const STICK_MIN_TOKENS: u64 = 20_000;
/// Warn about an expiring window when this little of it is left.
const WARN_BELOW_MS: u64 = 60_000;
/// A turn whose cache hit rate is below this (percent) counts as a miss.
const MISS_BELOW_PERCENT: u64 = 50;

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

/// `/cache`: the dashboard.
pub fn dashboard(log: &SessionLog, warm: Option<&WarmCache>, now_ms: u64, style: Style) -> String {
    let rows = by_provider(log);
    if rows.is_empty() {
        return format!("  {}", style.dim("No cache data yet: no turn in this session reported token usage."));
    }
    let mut lines = vec![style.bold("Cache by provider")];
    for r in &rows {
        lines.push(format!(
            "  {:<10}  hit {:>3}%  read {:>6}  rebuilt {:>6}  saved ~{} tokens of input cost",
            r.provider,
            r.hit_percent(),
            human_count(r.read),
            human_count(r.rebuilt),
            human_count(r.saved_tokens())
        ));
    }
    if let Some(w) = warm {
        let state = countdown(w, now_ms).unwrap_or_else(|| format!("{} has no documented cache window", w.provider));
        lines.push(String::new());
        lines.push(format!("  {} {} tokens cached; {state}", style.bold("Warm now"), human_count(w.tokens)));
        if let Some(warning) = expiry_warning(w, now_ms) {
            lines.push(format!("  {} {warning}", style.yellow("⚠")));
        }
    }
    let misses: Vec<TurnCache> = per_turn(log).into_iter().filter(|t| t.miss.is_some()).collect();
    if !misses.is_empty() {
        lines.push(String::new());
        lines.push(style.bold("Cache misses"));
        for m in misses.iter().rev().take(8).rev() {
            let why = m.miss.map_or("", MissCause::label);
            lines.push(format!("  turn {:<3} {}  hit {:>3}%  rebuilt {:>6}  {}", m.turn, m.model, m.hit_percent(), human_count(m.rebuilt), style.yellow(why)));
        }
    }
    lines.join("\n")
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
    use crate::turn::TurnStatus;

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

    #[test]
    fn dashboard_shows_providers_warmth_and_misses() {
        let log = log_of(vec![rec(ProviderId::Claude, "sonnet", 40_000, 0, 1_000), rec(ProviderId::Claude, "sonnet", 45_000, 40_000, 5_000)]);
        let w = WarmCache::after_turn(ProviderId::Claude, &usage(45_000, 40_000), 5_000);
        let text = dashboard(&log, Some(&w), 5_000 + 250_000, Style::off());
        assert!(text.contains("Cache by provider") && text.contains("hit  47%"), "{text}");
        assert!(text.contains("Warm now") && text.contains("expires in 50s"), "{text}");
        assert!(text.contains("Cache misses") && text.contains("turn 1") && text.contains("first turn"), "{text}");
        assert!(dashboard(&SessionLog::default(), None, 0, Style::off()).contains("No cache data yet"));
    }
}
