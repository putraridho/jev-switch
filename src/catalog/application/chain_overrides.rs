//! The user's own model chains: reads the setting that names the file, reads the file and
//! hands what it says to the catalog.

use crate::catalog::domain::model_table::{Overrides, RawChain, RawEntry};
use crate::shared::application::host::Host;

/// [`from_setting`] for the file named by `JEV_CHAINS`.
pub fn from_host(host: &dyn Host) -> Overrides {
    from_setting(host.env("JEV_CHAINS"))
}

/// Read the file named by the `JEV_CHAINS` setting (`setting`), if set. A missing or broken
/// file yields the defaults plus a note, so a typo never stops routing.
pub fn from_setting(setting: Option<String>) -> Overrides {
    let Some(path) = setting.filter(|p| !p.is_empty()) else {
        return Overrides::default();
    };
    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| load_overrides(&t)) {
        Ok(o) => o,
        Err(e) => Overrides::ignored(&e),
    }
}

/// Parse `{ "feature": ["claude:claude-sonnet-5-5", ...], ... }`. The caller reads the
/// file. Only text that is not a JSON object is an error; see [`Overrides::from_raw`] for
/// what is skipped.
pub fn load_overrides(json: &str) -> Result<Overrides, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let obj = value.as_object().ok_or("expected a JSON object of task type to list")?;
    let raw = obj
        .iter()
        .map(|(key, list)| {
            let chain = match list.as_array() {
                Some(items) => RawChain::List(
                    items
                        .iter()
                        .map(|item| match item.as_str() {
                            Some(s) => RawEntry::Ref(s.to_string()),
                            None => RawEntry::NotAString(item.to_string()),
                        })
                        .collect(),
                ),
                None => RawChain::NotAList,
            };
            (key.clone(), chain)
        })
        .collect();
    Ok(Overrides::from_raw(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chains_file_is_read_with_its_unknowns_reported() {
        let o = load_overrides(
            r#"{"feature":["claude:claude-haiku-5-5","claude:nope",3,"grok-4.7"],"weird":["claude:claude-opus-5-5"],"docs":"x"}"#,
        )
        .unwrap();
        assert_eq!(o.notes.len(), 5, "{:?}", o.notes);
        assert_eq!(o.get(crate::catalog::domain::model_table::TaskType::Feature).map(<[_]>::len), Some(1));
        assert!(load_overrides("[1]").is_err());
        assert!(load_overrides("not json").is_err());
    }
}
