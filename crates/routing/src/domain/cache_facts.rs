//! What is known about a prompt cache, per (provider, model, effective backend).
//!
//! Nothing here is a price list or a promise. The built-in numbers (how long a provider keeps a
//! prefix, what a cached read costs) are priors, and every figure says whether it is `assumed`
//! (a prior nobody has contradicted) or `observed` (counted from turns whose usage was
//! reported). Observed facts are a running estimate with their sample count, so a reader can see
//! how much is behind them. The facts are advisory: routing may use them to prefer a warm model,
//! never to override a quality floor, a hard rule, quota steering or a pin.
//!
//! The same model on a custom endpoint and on its native login is two different caches, so the
//! backend's identity is part of the key. A file keeps the facts between sessions: counts and
//! gaps only, never prompt text.

use std::collections::BTreeMap;

use jev_shared::{ ProviderId, Usage };

/// How long a provider documents keeping a prefix, when it does (Anthropic's ephemeral cache
/// lasts 5 minutes and every hit refreshes it; the others publish similar windows). A prior.
pub fn prior_ttl_ms(provider: ProviderId) -> Option<u64> {
    match provider {
        ProviderId::Claude | ProviderId::Codex | ProviderId::Grok => Some(300_000),
        ProviderId::DeepSeek | ProviderId::Gemini => None,
    }
}

/// The lifetime assumed for a provider that documents none: short, so an unknown cache is never
/// treated as warm for long. A hit observed after a longer gap raises it.
pub const UNKNOWN_TTL_MS: u64 = 60_000;
/// A turn whose cached share of its input is at least this (percent) counts as a cache hit.
const HIT_PERCENT: u64 = 10;
/// Past this many samples the older ones count for less (the counts are halved).
const SAMPLE_CAP: u32 = 64;
/// Facts not refreshed for this long are forgotten when the file is read or written.
pub const MAX_AGE_MS: u64 = 30 * 24 * 3_600_000;
/// The file in the state folder that keeps the facts.
pub const FILE: &str = "cache-facts.tsv";

/// Whether a figure is a built-in prior or counted from turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Assumed,
    Observed,
}

impl Basis {
    pub fn label(self) -> &'static str {
        match self {
            Self::Assumed => "assumed",
            Self::Observed => "observed",
        }
    }
}

/// How long a cache is believed to live after its last use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ttl {
    pub ms: u64,
    pub basis: Basis,
}

/// Whose cache it is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheKey {
    pub provider: ProviderId,
    pub model: String,
    /// The effective backend's identity (`Fundings::identity`): a custom endpoint is not the
    /// native login.
    pub backend: String,
}

impl CacheKey {
    pub fn new(provider: ProviderId, model: &str, backend: &str) -> Self {
        Self { provider, model: model.to_string(), backend: backend.to_string() }
    }
}

/// What turns on one key have shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheFacts {
    /// Input tokens over the counted turns (recent ones weigh more once there are many).
    pub input: u64,
    /// Of those, the tokens read from the cache.
    pub read: u64,
    pub samples: u32,
    /// The longest gap after which a turn still hit the cache: the lifetime is at least this.
    pub hit_after_ms: u64,
    /// The shortest gap, beyond every hit, after which a turn missed: the lifetime is probably
    /// below this (a miss can also be a changed prefix, so it only suggests).
    pub miss_after_ms: Option<u64>,
    pub updated_ms: u64,
}

impl CacheFacts {
    /// Count one finished turn. `gap_ms` is the time since the previous turn on this key left
    /// something cached (`None` when there was none, so the turn says nothing of the lifetime).
    /// A turn without reported counters changes nothing: no counter is not a zero counter.
    pub fn observe(&mut self, usage: &Usage, gap_ms: Option<u64>, now_ms: u64) -> bool {
        if !usage.counters_known() {
            return false;
        }
        let read = usage.cached_input_tokens.min(usage.input_tokens);
        self.input = self.input.saturating_add(usage.input_tokens);
        self.read = self.read.saturating_add(read);
        self.samples = self.samples.saturating_add(1);
        if self.samples > SAMPLE_CAP {
            (self.input, self.read, self.samples) = (
                self.input / 2,
                self.read / 2,
                self.samples / 2,
            );
        }
        self.updated_ms = now_ms;
        let Some(gap) = gap_ms else {
            return true;
        };
        if read * 100 >= usage.input_tokens * HIT_PERCENT {
            self.hit_after_ms = self.hit_after_ms.max(gap);
            self.miss_after_ms = self.miss_after_ms.filter(|m| *m > self.hit_after_ms);
        } else if gap > self.hit_after_ms {
            self.miss_after_ms = Some(self.miss_after_ms.map_or(gap, |m| m.min(gap)));
        }
        true
    }

    /// The share of input read from the cache, in percent; `None` before any counted turn.
    pub fn hit_percent(&self) -> Option<u64> {
        (self.samples > 0 && self.input > 0).then(|| (self.read * 100) / self.input)
    }

    /// How long the cache is believed to live: the provider's documented window (or a short
    /// one when it documents none), raised by a hit seen after a longer gap and lowered to just
    /// under a gap after which it missed. It is `observed` only when evidence moved it.
    pub fn ttl(&self, provider: ProviderId) -> Ttl {
        let mut ttl = Ttl {
            ms: prior_ttl_ms(provider).unwrap_or(UNKNOWN_TTL_MS),
            basis: Basis::Assumed,
        };
        if self.hit_after_ms > ttl.ms {
            ttl = Ttl { ms: self.hit_after_ms, basis: Basis::Observed };
        }
        if let Some(miss) = self.miss_after_ms.filter(|m| *m <= ttl.ms) {
            ttl = Ttl { ms: miss.saturating_sub(1).max(self.hit_after_ms), basis: Basis::Observed };
        }
        ttl
    }
}

/// The lifetime to believe when nothing has been counted for the key.
pub fn ttl_of(provider: ProviderId, facts: Option<&CacheFacts>) -> Ttl {
    facts.map_or_else(
        || CacheFacts::default().ttl(provider),
        |f| f.ttl(provider)
    )
}

/// The facts of every key seen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheBook(BTreeMap<CacheKey, CacheFacts>);

impl CacheBook {
    pub fn get(&self, key: &CacheKey) -> Option<&CacheFacts> {
        self.0.get(key)
    }

    /// Count a turn on `key`; see [`CacheFacts::observe`].
    pub fn observe(
        &mut self,
        key: &CacheKey,
        usage: &Usage,
        gap_ms: Option<u64>,
        now_ms: u64
    ) -> bool {
        if !usage.counters_known() {
            return false;
        }
        self.0.entry(key.clone()).or_default().observe(usage, gap_ms, now_ms)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&CacheKey, &CacheFacts)> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Both books, the record that changed last winning per key (another session may have saved).
    pub fn merged(&self, other: &Self) -> Self {
        let mut all = self.0.clone();
        for (key, facts) in &other.0 {
            if all.get(key).is_none_or(|mine| mine.updated_ms < facts.updated_ms) {
                all.insert(key.clone(), facts.clone());
            }
        }
        Self(all)
    }

    /// Without the facts nothing has refreshed for [`MAX_AGE_MS`].
    pub fn expired(mut self, now_ms: u64) -> Self {
        self.0.retain(|_, f| now_ms.saturating_sub(f.updated_ms) <= MAX_AGE_MS);
        self
    }

    /// Read the file: a line that does not parse is skipped, and so is a fact that expired.
    pub fn parse(text: &str, now_ms: u64) -> Self {
        let rows = text.lines().filter_map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            let [provider, model, backend, input, read, samples, hit_after, miss_after, updated] =
                c.as_slice() else {
                return None;
            };
            let (input, read) = (input.parse::<u64>().ok()?, read.parse::<u64>().ok()?);
            let facts = CacheFacts {
                input,
                read: read.min(input),
                samples: samples.parse().ok()?,
                hit_after_ms: hit_after.parse().ok()?,
                miss_after_ms: match *miss_after {
                    "-" => None,
                    n => Some(n.parse().ok()?),
                },
                updated_ms: updated.parse().ok()?,
            };
            Some((CacheKey::new(provider.parse().ok()?, model, backend), facts))
        });
        Self(rows.collect()).expired(now_ms)
    }

    /// The file's text: counts, gaps and names of models and backends, nothing typed or answered.
    pub fn file_text(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        self.0
            .iter()
            .map(|(k, f)| {
                let miss = f.miss_after_ms.map_or("-".to_string(), |m| m.to_string());
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    k.provider,
                    clean(&k.model),
                    clean(&k.backend),
                    f.input,
                    f.read,
                    f.samples,
                    f.hit_after_ms,
                    miss,
                    f.updated_ms
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::Measurement;
    use ProviderId::{ Claude, DeepSeek, Gemini };

    fn usage(input: u64, cached: u64) -> Usage {
        Usage {
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: 1,
            ..Usage::default()
        }
    }

    fn key(provider: ProviderId) -> CacheKey {
        CacheKey::new(provider, "m", "b")
    }

    #[test]
    fn the_ratio_is_counted_from_reported_turns_with_its_sample_count() {
        let mut f = CacheFacts::default();
        assert_eq!(f.hit_percent(), None);
        assert!(f.observe(&usage(1_000, 0), None, 1));
        assert!(f.observe(&usage(3_000, 3_000), Some(10_000), 2));
        assert_eq!((f.samples, f.hit_percent()), (2, Some(75)));
        assert_eq!(f.updated_ms, 2);
    }

    #[test]
    fn a_turn_without_reported_counters_is_not_a_zero() {
        let mut book = CacheBook::default();
        let unreported = Usage {
            measurement: Measurement::Unavailable,
            input_tokens: 5_000,
            ..Usage::default()
        };
        assert!(!book.observe(&key(Claude), &unreported, Some(1), 1));
        assert!(!book.observe(&key(Claude), &Usage::default(), Some(1), 1), "no input at all");
        assert!(book.is_empty());
    }

    #[test]
    fn the_estimate_keeps_recent_turns_heavier_once_there_are_many() {
        let mut f = CacheFacts::default();
        for _ in 0..200 {
            f.observe(&usage(1_000, 0), None, 1);
        }
        assert!(f.samples <= SAMPLE_CAP, "{}", f.samples);
        for _ in 0..40 {
            f.observe(&usage(1_000, 1_000), None, 2);
        }
        assert!(
            f.hit_percent().unwrap() >= 30,
            "recent hits move a long-run zero: {:?}",
            f.hit_percent()
        );
    }

    #[test]
    fn documented_windows_are_assumed_and_unknown_ones_are_short() {
        let f = CacheFacts::default();
        assert_eq!(f.ttl(Claude), Ttl { ms: 300_000, basis: Basis::Assumed });
        assert_eq!(f.ttl(Gemini), Ttl { ms: UNKNOWN_TTL_MS, basis: Basis::Assumed });
        assert_eq!(f.ttl(DeepSeek).ms, UNKNOWN_TTL_MS, "never warm forever");
        assert_eq!(ttl_of(Gemini, None), f.ttl(Gemini));
    }

    #[test]
    fn a_hit_after_a_gap_proves_the_lifetime_is_at_least_that_long() {
        let mut f = CacheFacts::default();
        f.observe(&usage(10_000, 9_000), Some(600_000), 1);
        assert_eq!(f.ttl(Gemini), Ttl { ms: 600_000, basis: Basis::Observed });
        assert_eq!(
            f.ttl(Claude),
            Ttl { ms: 600_000, basis: Basis::Observed },
            "even above the documented window"
        );
        f.observe(&usage(10_000, 9_000), Some(30_000), 2);
        assert_eq!(f.ttl(Gemini).ms, 600_000, "a shorter hit proves less");
    }

    #[test]
    fn a_miss_after_a_gap_suggests_a_shorter_lifetime_than_assumed() {
        let mut f = CacheFacts::default();
        f.observe(&usage(10_000, 0), Some(120_000), 1);
        assert_eq!(f.ttl(Claude), Ttl { ms: 119_999, basis: Basis::Observed });
        assert_eq!(f.ttl(Gemini).ms, 60_000, "already shorter than the miss: unchanged");
        assert_eq!(f.ttl(Gemini).basis, Basis::Assumed);
        // A miss inside a gap that is already known to hit is a changed prefix, not expiry.
        f.observe(&usage(10_000, 9_000), Some(200_000), 2);
        f.observe(&usage(10_000, 0), Some(150_000), 3);
        assert_eq!(f.miss_after_ms, None, "contradicted by the longer hit");
        assert_eq!(f.ttl(Claude).ms, 300_000);
    }

    #[test]
    fn a_turn_with_no_earlier_cache_says_nothing_of_the_lifetime() {
        let mut f = CacheFacts::default();
        f.observe(&usage(10_000, 0), None, 1);
        assert_eq!((f.hit_after_ms, f.miss_after_ms), (0, None));
    }

    #[test]
    fn the_key_names_model_and_effective_backend() {
        let mut book = CacheBook::default();
        let native = CacheKey::new(Claude, "sonnet", "Claude login");
        let custom = CacheKey::new(Claude, "sonnet", "custom endpoint (billing unknown)");
        let other_model = CacheKey::new(Claude, "opus", "Claude login");
        book.observe(&native, &usage(1_000, 900), None, 1);
        assert!(book.get(&custom).is_none() && book.get(&other_model).is_none());
        book.observe(&custom, &usage(1_000, 0), None, 1);
        assert_eq!(book.get(&native).unwrap().hit_percent(), Some(90));
        assert_eq!(book.get(&custom).unwrap().hit_percent(), Some(0));
    }

    #[test]
    fn the_file_round_trips_and_forgets_old_or_garbled_facts() {
        let mut book = CacheBook::default();
        book.observe(&key(Claude), &usage(2_000, 1_500), None, 1_000);
        book.observe(&key(Claude), &usage(2_000, 1_900), Some(90_000), 2_000);
        book.observe(&key(Gemini), &usage(100, 0), Some(5_000), 1_000);
        let text = book.file_text();
        assert!(!text.contains("prompt"), "{text}");
        assert_eq!(CacheBook::parse(&text, 3_000), book);
        let later = CacheBook::parse(&text, 2_000 + MAX_AGE_MS + 1);
        assert!(later.is_empty(), "everything is older than the limit");
        let dirty = format!(
            "{text}not a row\nclaude\tm\tb\tx\t1\t1\t0\t-\t5\nnobody\tm\tb\t1\t1\t1\t0\t-\t5\n"
        );
        assert_eq!(CacheBook::parse(&dirty, 3_000), book);
        assert!(CacheBook::parse("", 0).is_empty());
    }

    #[test]
    fn merging_keeps_the_record_that_changed_last() {
        let (mut a, mut b) = (CacheBook::default(), CacheBook::default());
        a.observe(&key(Claude), &usage(1_000, 0), None, 10);
        b.observe(&key(Claude), &usage(1_000, 1_000), None, 20);
        b.observe(&key(Gemini), &usage(1_000, 500), None, 5);
        let merged = a.merged(&b);
        assert_eq!(merged.get(&key(Claude)).unwrap().hit_percent(), Some(100));
        assert!(merged.get(&key(Gemini)).is_some());
        assert_eq!(b.merged(&a).get(&key(Claude)).unwrap().hit_percent(), Some(100));
    }
}
