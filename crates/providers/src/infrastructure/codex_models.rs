//! The context windows Codex itself reports for the models its configured backend serves
//! (`$CODEX_HOME/models_cache.json`), so routing does not assume one fixed size for models
//! whose real window depends on that backend.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use jev_shared::ProviderId;

#[derive(Deserialize)]
struct Cache {
    #[serde(default)]
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    slug: String,
    context_window: Option<u64>,
    /// The share of the window a session may really fill; Codex reserves the rest.
    effective_context_window_percent: Option<u64>,
}

impl Model {
    /// The tokens a session may fill, or `None` when the entry does not say sensibly.
    fn usable_window(&self) -> Option<u64> {
        let window = self.context_window.filter(|w| *w > 0)?;
        let percent = self.effective_context_window_percent.unwrap_or(100);
        if !(1..=100).contains(&percent) {
            return None;
        }
        Some(window / 100 * percent + window % 100 * percent / 100)
    }
}

/// Per Codex model, the window the cache reports; nothing for a missing, unreadable or
/// garbled file, and nothing for an entry without a sensible window.
pub fn windows_in(path: &Path) -> HashMap<(ProviderId, String), u64> {
    let Some(cache) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Cache>(&t).ok()) else {
        return HashMap::new();
    };
    cache.models.into_iter().filter_map(|m| Some(((ProviderId::Codex, m.slug.clone()), m.usable_window()?))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> std::path::PathBuf {
        let path = jev_testkit::scratch_dir::scratch_dir("codex-models", name).join("models_cache.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn the_usable_share_of_each_reported_window_is_kept() {
        let path = file(
            "ok",
            r#"{"fetched_at":"x","models":[
                {"slug":"gpt-6.1-sol","context_window":922000,"effective_context_window_percent":95},
                {"slug":"plain","context_window":200000},
                {"slug":"zero","context_window":0},
                {"slug":"odd","context_window":1000,"effective_context_window_percent":0},
                {"slug":"huge","context_window":1000,"effective_context_window_percent":500},
                {"slug":"none"}]}"#,
        );
        let w = windows_in(&path);
        assert_eq!(w[&(ProviderId::Codex, "gpt-6.1-sol".to_string())], 875_900);
        assert_eq!(w[&(ProviderId::Codex, "plain".to_string())], 200_000);
        assert_eq!(w.len(), 2, "{w:?}");
    }

    #[test]
    fn a_missing_or_garbled_cache_reports_nothing() {
        assert!(windows_in(Path::new("/nonexistent/models_cache.json")).is_empty());
        assert!(windows_in(&file("bad", "not json")).is_empty());
        assert!(windows_in(&file("shape", r#"{"models":"x"}"#)).is_empty());
    }
}
