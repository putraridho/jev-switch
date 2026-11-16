//! Remembering which providers hit their usage limit, so the router skips them
//! until they reset instead of paying a failed turn to rediscover it.
//!
//! Time is always passed in (`now_ms`) and the file path is always a parameter, so
//! every rule here is testable without a clock or the real state directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::backend::ProviderId;

const MINUTE_MS: u64 = 60_000;
/// Used when the CLI gave no reset time we can read. An hour is long enough not to
/// hammer a limited provider and short enough to recover the same day.
const DEFAULT_MS: u64 = 60 * MINUTE_MS;
/// Never trust a parsed reset to be shorter than this (clock skew, "resets now").
const MIN_MS: u64 = MINUTE_MS;
/// Never wait longer than the longest plan window (weekly limits), so one bad
/// timestamp cannot disable a provider for months.
const MAX_MS: u64 = 8 * 24 * 60 * MINUTE_MS;
/// Epoch values at or above this are milliseconds; below it, seconds. 1e11 s is the
/// year 5138 and 1e11 ms is 1973, so real timestamps never straddle the line.
const MS_THRESHOLD: u64 = 100_000_000_000;

/// Milliseconds a provider should be skipped, given the CLI's reset text.
/// Understands only epoch seconds or milliseconds as digits; anything else (or
/// nothing) means [`DEFAULT_MS`]. The result is clamped to [1 minute, 8 days].
pub fn cooldown_ms(resets_at: Option<&str>, now_ms: u64) -> u64 {
    let reset_ms = resets_at
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| {
            let n: u64 = s.parse().ok()?;
            Some(if n >= MS_THRESHOLD { n } else { n.saturating_mul(1000) })
        });
    let wait = reset_ms.map_or(DEFAULT_MS, |r| r.saturating_sub(now_ms));
    wait.clamp(MIN_MS, MAX_MS)
}

/// When each provider's usage limit resets (epoch ms).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cooldowns {
    until_ms: HashMap<ProviderId, u64>,
}

impl Cooldowns {
    /// Start (or restart) a cooldown after a quota error.
    pub fn start(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.until_ms.insert(p, now_ms.saturating_add(cooldown_ms(resets_at, now_ms)));
    }

    /// Milliseconds left if the provider is cooling down, else `None`.
    pub fn active(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.until_ms.get(&p).and_then(|u| u.checked_sub(now_ms)).filter(|left| *left > 0)
    }

    /// Forget a cooldown, e.g. after a successful turn proves the provider is back.
    pub fn clear(&mut self, p: ProviderId) {
        self.until_ms.remove(&p);
    }

    /// `{"claude": 1760000000000, ...}`: provider name to reset time.
    pub fn to_json(&self) -> Value {
        Value::Object(self.until_ms.iter().map(|(p, u)| (p.as_str().to_string(), Value::from(*u))).collect::<Map<_, _>>())
    }

    /// Tolerant reader: anything that is not a provider name with a number is skipped,
    /// so a damaged file costs at most the broken entries.
    pub fn from_json(v: &Value) -> Self {
        let until_ms = v
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.parse::<ProviderId>().ok()?, v.as_u64()?)))
            .collect();
        Self { until_ms }
    }

    /// Read `path`; a missing or garbled file is an empty set.
    pub fn load(path: &Path) -> Self {
        let value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok());
        value.map_or_else(Self::default, |v| Self::from_json(&v))
    }

    /// Write atomically (temp file in the same directory, then rename) so a crash
    /// mid-write never leaves a half-written file for the next start to choke on.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, self.to_json().to_string())?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

/// `<state_dir>/cooldowns.json`, or `None` if there is no state directory.
pub fn default_path() -> Option<PathBuf> {
    Some(crate::log::state_dir()?.join("cooldowns.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::{Claude, Codex};

    const NOW: u64 = 1_760_000_000_000;

    #[test]
    fn default_is_one_hour() {
        for text in [None, Some(""), Some("tomorrow 9am"), Some("12abc")] {
            assert_eq!(cooldown_ms(text, NOW), DEFAULT_MS, "{text:?}");
        }
    }

    #[test]
    fn parses_epoch_seconds_and_milliseconds() {
        let secs = ((NOW + 2 * 60 * MINUTE_MS) / 1000).to_string();
        let ms = (NOW + 2 * 60 * MINUTE_MS).to_string();
        assert_eq!(cooldown_ms(Some(&secs), NOW), 120 * MINUTE_MS);
        assert_eq!(cooldown_ms(Some(&format!(" {ms} ")), NOW), 120 * MINUTE_MS);
    }

    #[test]
    fn clamps_to_one_minute_and_eight_days() {
        let past = (NOW / 1000 - 5000).to_string();
        assert_eq!(cooldown_ms(Some(&past), NOW), MIN_MS);
        let far = (NOW / 1000 + 90 * 86_400).to_string();
        assert_eq!(cooldown_ms(Some(&far), NOW), MAX_MS);
        assert_eq!(cooldown_ms(Some("99999999999999999999999"), NOW), DEFAULT_MS); // overflows u64
    }

    #[test]
    fn active_counts_down_then_expires() {
        let mut c = Cooldowns::default();
        assert_eq!(c.active(Claude, NOW), None);
        c.start(Claude, None, NOW);
        assert_eq!(c.active(Claude, NOW), Some(DEFAULT_MS));
        assert_eq!(c.active(Claude, NOW + DEFAULT_MS - 1), Some(1));
        assert_eq!(c.active(Claude, NOW + DEFAULT_MS), None);
        assert_eq!(c.active(Codex, NOW), None);
    }

    #[test]
    fn clear_removes() {
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.clear(Claude);
        assert_eq!(c.active(Claude, NOW), None);
    }

    #[test]
    fn json_round_trip_and_garbage() {
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.start(Codex, Some(&(NOW / 1000 + 7200).to_string()), NOW);
        assert_eq!(Cooldowns::from_json(&c.to_json()), c);
        for garbage in [serde_json::json!(null), serde_json::json!([1, 2]), serde_json::json!("x")] {
            assert_eq!(Cooldowns::from_json(&garbage), Cooldowns::default());
        }
        let partly = serde_json::json!({"claude": 5, "nobody": 6, "codex": "soon", "gemini": -1});
        let c = Cooldowns::from_json(&partly);
        assert_eq!(c.until_ms.len(), 1);
        assert_eq!(c.until_ms[&Claude], 5);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-cooldown-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_then_load_survives_a_restart_and_leaves_no_temp_file() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("cooldowns.json");
        let mut c = Cooldowns::default();
        c.start(Claude, None, NOW);
        c.save(&path).unwrap();
        assert_eq!(Cooldowns::load(&path), c);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_tolerates_missing_and_corrupt_files() {
        let dir = temp_dir("corrupt");
        assert_eq!(Cooldowns::load(&dir.join("absent.json")), Cooldowns::default());
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert_eq!(Cooldowns::load(&bad), Cooldowns::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
