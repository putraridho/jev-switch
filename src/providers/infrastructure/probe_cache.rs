//! The last provider probe, kept on disk so the next start does not wait for the provider
//! CLIs (a probe runs five of them and takes up to 5 s; a cache read takes well under a
//! millisecond). A cached answer is only used while it is fresh ([`TTL_MS`]), was made with
//! the same `PATH` and provider settings ([`fingerprint`]), and says something definite for
//! every provider. The session then probes again in the background and corrects the screen
//! if anything changed, so a stale answer lives for a moment, never for a session.
//!
//! It stores state, plan, models and the probe's own short words; no keys, no account names.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::providers::domain::availability::{Availability, State};
use crate::backend::ProviderId;
use crate::providers::application::provider_registry::ALL_PROVIDERS;

/// How long a probe answer may be reused.
pub const TTL_MS: u64 = 10 * 60 * 1000;

const FILE_NAME: &str = "probe-cache.json";

/// The settings a probe's answer depends on. A different value means a different machine
/// as far as the probe is concerned, so the cache is not used.
const FINGERPRINT_VARS: [&str; 6] = ["PATH", "HOME", "CLAUDE_BIN", "CODEX_BIN", "GROK_BIN", "GEMINI_BIN"];

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    provider: String,
    state: State,
    plan: Option<String>,
    models: Option<Vec<String>>,
    detail: String,
    verified: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    saved_ms: u64,
    fingerprint: u64,
    entries: Vec<Entry>,
}

/// Where the cache lives, when there is a state directory.
pub fn default_path() -> Option<PathBuf> {
    crate::shared::infrastructure::jsonl_log::state_dir().map(|d| d.join(FILE_NAME))
}

/// Whether the user turned the cache off with `JEV_PROBE_CACHE=off`.
pub fn disabled(setting: Option<&str>) -> bool {
    matches!(setting.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("off" | "0" | "false" | "no"))
}

/// A number that changes when what a probe would find changes. `lookup` reads one setting.
/// Whether a DeepSeek key exists matters (it decides that provider), its value does not
/// and is never hashed.
pub fn fingerprint(lookup: &dyn Fn(&str) -> Option<String>) -> u64 {
    let mut h = DefaultHasher::new();
    for name in FINGERPRINT_VARS {
        name.hash(&mut h);
        lookup(name).hash(&mut h);
    }
    lookup("DEEPSEEK_API_KEY").is_some_and(|k| !k.trim().is_empty()).hash(&mut h);
    h.finish()
}

/// The cached answers, when they may be used right now.
pub fn load(path: &Path, now_ms: u64, fingerprint: u64) -> Option<Vec<Availability>> {
    let file: CacheFile = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let fresh = now_ms.saturating_sub(file.saved_ms) < TTL_MS && file.saved_ms <= now_ms.saturating_add(60_000);
    if !fresh || file.fingerprint != fingerprint {
        return None;
    }
    let avail: Vec<Availability> = file
        .entries
        .into_iter()
        .filter_map(|e| {
            Some(Availability { provider: e.provider.parse::<ProviderId>().ok()?, state: e.state, plan: e.plan, models: e.models, detail: e.detail, verified: e.verified })
        })
        .collect();
    let complete = ALL_PROVIDERS.iter().all(|p| avail.iter().any(|a| a.provider == *p));
    (complete && avail.iter().all(|a| a.state != State::Unknown)).then_some(avail)
}

/// Remember `avail` for the next start. Nothing is written when any provider could not be
/// told (an Unknown answer is not worth reusing). Written to a temporary file and renamed,
/// so a reader never sees half a file. Best effort: a failure only costs the speed-up.
pub fn save(path: &Path, now_ms: u64, fingerprint: u64, avail: &[Availability]) {
    if avail.iter().any(|a| a.state == State::Unknown) {
        return;
    }
    let entries = avail
        .iter()
        .map(|a| Entry { provider: a.provider.as_str().to_string(), state: a.state, plan: a.plan.clone(), models: a.models.clone(), detail: a.detail.clone(), verified: a.verified })
        .collect();
    let Ok(text) = serde_json::to_string(&CacheFile { saved_ms: now_ms, fingerprint, entries }) else { return };
    // Owner-only, like every other file in the state folder.
    let _ = crate::shared::infrastructure::private_file::write_atomic(path, &text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(state: State) -> Vec<Availability> {
        ALL_PROVIDERS
            .iter()
            .map(|p| Availability { provider: *p, state, plan: Some("max".into()), models: Some(vec!["m".into()]), detail: "ok".into(), verified: true })
            .collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-probecache-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn a_saved_probe_is_read_back_while_fresh() {
        let path = temp("roundtrip");
        save(&path, 1_000, 7, &all(State::Ready));
        assert_eq!(load(&path, 1_000 + TTL_MS - 1, 7), Some(all(State::Ready)));
    }

    #[test]
    fn an_old_cache_or_one_from_another_setup_is_not_used() {
        let path = temp("stale");
        save(&path, 1_000, 7, &all(State::Ready));
        assert_eq!(load(&path, 1_000 + TTL_MS, 7), None, "too old");
        assert_eq!(load(&path, 2_000, 8), None, "different PATH or settings");
        assert_eq!(load(&path, 0, 7), Some(all(State::Ready)), "a small clock step back is tolerated");
        let future = temp("future");
        save(&future, 10_000_000, 7, &all(State::Ready));
        assert_eq!(load(&future, 1_000, 7), None, "a clock far in the past must not make it eternal");
    }

    #[test]
    fn unknown_answers_are_never_cached() {
        let path = temp("unknown");
        let mut avail = all(State::Ready);
        avail[2].state = State::Unknown;
        save(&path, 1_000, 7, &avail);
        assert_eq!(load(&path, 1_001, 7), None);
    }

    #[test]
    fn a_missing_provider_or_a_broken_file_is_a_miss_not_a_crash() {
        let path = temp("broken");
        assert_eq!(load(&path, 1, 7), None);
        assert!(std::fs::write(&path, "{not json").is_ok());
        assert_eq!(load(&path, 1, 7), None);
        save(&path, 1_000, 7, &all(State::Ready)[..4]);
        assert_eq!(load(&path, 1_001, 7), None, "an incomplete list is not enough to route on");
    }

    #[test]
    fn the_fingerprint_follows_path_and_key_presence_but_never_the_key() {
        let with = |path: &'static str, key: Option<&'static str>| {
            fingerprint(&move |name| match name {
                "PATH" => Some(path.to_string()),
                "DEEPSEEK_API_KEY" => key.map(String::from),
                _ => None,
            })
        };
        assert_eq!(with("/a", Some("k1")), with("/a", Some("k2")));
        assert_ne!(with("/a", None), with("/a", Some("k1")));
        assert_ne!(with("/a", None), with("/b", None));
    }

    #[test]
    fn the_cache_can_be_turned_off() {
        assert!(disabled(Some("off")) && disabled(Some(" OFF ")) && disabled(Some("0")));
        assert!(!disabled(None) && !disabled(Some("on")));
    }
}
