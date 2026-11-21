use std::collections::HashMap;

use serde_json::Value;

use super::models::find;
use super::{ModelSpec, TaskType};
use crate::shared::ProviderId;

/// User-supplied chains that replace the defaults for the task types they list.
#[derive(Debug, Default)]
pub struct Overrides {
    chains: HashMap<TaskType, Vec<&'static ModelSpec>>,
    /// Things that were ignored (unknown task, provider or model id), for the caller to show.
    pub notes: Vec<String>,
}

impl Overrides {
    /// Read the file named by `JEV_CHAINS`, if set. A missing or broken file yields
    /// the defaults plus a note, so a typo never stops routing.
    pub fn from_env() -> Self {
        let Some(path) = crate::shared::infrastructure::env_file::lookup("JEV_CHAINS").filter(|p| !p.is_empty()) else {
            return Self::default();
        };
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| load_overrides(&t)) {
            Ok(o) => o,
            Err(e) => Self { notes: vec![format!("JEV_CHAINS ignored: {e}")], ..Self::default() },
        }
    }

    /// The override for `task`, if the user listed one.
    pub fn get(&self, task: TaskType) -> Option<&[&'static ModelSpec]> {
        self.chains.get(&task).map(Vec::as_slice)
    }
}

/// Parse `{ "feature": ["claude:claude-sonnet-5-5", ...], ... }`. Pure: the caller
/// reads the file. Unknown task types, providers and model ids are skipped and
/// reported in [`Overrides::notes`]; only text that is not a JSON object is an error.
pub fn load_overrides(json: &str) -> Result<Overrides, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("not valid JSON: {e}"))?;
    let obj = value.as_object().ok_or("expected a JSON object of task type to list")?;
    let mut out = Overrides::default();
    for (key, list) in obj {
        let Ok(task) = key.parse::<TaskType>() else {
            out.notes.push(format!("unknown task type `{key}` ignored"));
            continue;
        };
        let Some(items) = list.as_array() else {
            out.notes.push(format!("`{key}` must be a list; ignored"));
            continue;
        };
        let mut chain = Vec::new();
        for item in items {
            match item.as_str().ok_or_else(|| format!("{item} is not a string")).and_then(parse_model_ref) {
                Ok(spec) => chain.push(spec),
                Err(e) => out.notes.push(format!("`{key}`: {e}; ignored")),
            }
        }
        out.chains.insert(task, chain);
    }
    Ok(out)
}

/// `"<provider>:<model id>"` to a catalog entry.
fn parse_model_ref(s: &str) -> Result<&'static ModelSpec, String> {
    let (provider, id) = s.split_once(':').ok_or_else(|| format!("`{s}` is not <provider>:<model id>"))?;
    let provider: ProviderId = provider.parse()?;
    find(provider, id.trim()).ok_or_else(|| format!("unknown model `{s}`"))
}
