//! The last plan reading each provider gave, kept across runs so the router row shows
//! "how much of my plan is used" at startup instead of only after the first turn.
//!
//! One small JSON file in the state directory, `{"claude": {window, utilization, resets_at}}`.
//! A reading whose window has already reset says nothing about now, so [`is_current`]
//! drops it rather than show a stale number.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use jev_backend::infrastructure::plan_usage_file::PlanUsageFile;
use jev_backend::{PlanUsage, ProviderId};

/// Where readings are saved; `None` when there is no state directory.
pub fn default_path() -> Option<PathBuf> {
    jev_shared::infrastructure::jsonl_log::state_dir().map(|d| d.join("plan-usage.json"))
}

/// Readings saved by earlier runs. A missing or broken file, or an unknown provider
/// name, is skipped: this is a display hint, never worth failing startup over.
pub fn load(path: &Path) -> HashMap<ProviderId, PlanUsage> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(saved) = serde_json::from_str::<BTreeMap<String, PlanUsageFile>>(&text) else { return HashMap::new() };
    saved.into_iter().filter_map(|(name, plan)| Some((name.parse::<ProviderId>().ok()?, PlanUsage::from(plan)))).collect()
}

/// Save every reading. Written to a temporary file and renamed, so a reader in another
/// process (`delegate_mcp`) never sees half a file. Errors are ignored for the same
/// reason [`load`] skips them.
pub fn save(path: &Path, plans: &HashMap<ProviderId, PlanUsage>) {
    let out: BTreeMap<String, PlanUsageFile> = plans.iter().map(|(p, plan)| (p.to_string(), PlanUsageFile::from(plan))).collect();
    let Ok(text) = serde_json::to_string_pretty(&out) else { return };
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Whether `plan` still describes the current window at `now_s` (unix seconds): true
/// until its reset time passes, and always when the CLI gave no reset time.
pub fn is_current(plan: &PlanUsage, now_s: u64) -> bool {
    plan.resets_at.is_none_or(|t| t > now_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(u: f64, resets_at: Option<u64>) -> PlanUsage {
        PlanUsage { window: "five_hour".into(), utilization: u, resets_at }
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-planstore-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("plan-usage.json")
    }

    #[test]
    fn saved_readings_load_back() {
        let path = temp_file("roundtrip");
        let plans = HashMap::from([(ProviderId::Claude, plan(0.32, Some(10))), (ProviderId::Codex, plan(0.21, None))]);
        save(&path, &plans);
        assert_eq!(load(&path), plans);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_broken_or_unknown_entries_are_skipped() {
        let path = temp_file("broken");
        assert!(load(&path).is_empty());
        std::fs::write(&path, "{not json").unwrap();
        assert!(load(&path).is_empty());
        std::fs::write(&path, r#"{"nobody":{"window":"w","utilization":0.5,"resets_at":null},"codex":{"window":"w","utilization":0.5,"resets_at":null}}"#).unwrap();
        assert_eq!(load(&path).keys().copied().collect::<Vec<_>>(), vec![ProviderId::Codex]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_reading_expires_when_its_window_resets() {
        assert!(is_current(&plan(0.9, Some(100)), 99));
        assert!(!is_current(&plan(0.9, Some(100)), 100));
        assert!(is_current(&plan(0.9, None), u64::MAX));
    }
}
