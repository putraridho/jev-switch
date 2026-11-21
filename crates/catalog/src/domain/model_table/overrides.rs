use std::collections::HashMap;

use super::models::find;
use super::{ModelSpec, TaskType};
use jev_shared::ProviderId;

/// User-supplied chains that replace the defaults for the task types they list.
#[derive(Debug, Default)]
pub struct Overrides {
    chains: HashMap<TaskType, Vec<&'static ModelSpec>>,
    /// Things that were ignored (unknown task, provider or model id), for the caller to show.
    pub notes: Vec<String>,
}

/// One item of a user's chain list, as read from the chains file.
#[derive(Debug, Clone, PartialEq)]
pub enum RawEntry {
    /// A string item, expected to read `<provider>:<model id>`.
    Ref(String),
    /// An item that is not a string; carries how to show it in the note.
    NotAString(String),
}

/// What the chains file gave for one task type.
#[derive(Debug, Clone, PartialEq)]
pub enum RawChain {
    List(Vec<RawEntry>),
    NotAList,
}

impl RawChain {
    /// A list of `<provider>:<model id>` strings.
    pub fn of(refs: &[&str]) -> Self {
        Self::List(refs.iter().map(|r| RawEntry::Ref((*r).to_string())).collect())
    }
}

impl Overrides {
    /// Turn what the chains file said, task type name to list, in file order, into overrides.
    /// Unknown task types, providers and model ids are skipped and reported in [`Overrides::notes`].
    pub fn from_raw(raw: Vec<(String, RawChain)>) -> Self {
        let mut out = Self::default();
        for (key, list) in raw {
            let Ok(task) = key.parse::<TaskType>() else {
                out.notes.push(format!("unknown task type `{key}` ignored"));
                continue;
            };
            let RawChain::List(items) = list else {
                out.notes.push(format!("`{key}` must be a list; ignored"));
                continue;
            };
            let mut chain = Vec::new();
            for item in items {
                match match item {
                    RawEntry::Ref(s) => parse_model_ref(&s),
                    RawEntry::NotAString(shown) => Err(format!("{shown} is not a string")),
                } {
                    Ok(spec) => chain.push(spec),
                    Err(e) => out.notes.push(format!("`{key}`: {e}; ignored")),
                }
            }
            out.chains.insert(task, chain);
        }
        out
    }

    /// Defaults plus one note: the setting could not be used.
    pub fn ignored(why: &str) -> Self {
        Self { notes: vec![format!("JEV_CHAINS ignored: {why}")], ..Self::default() }
    }

    /// The override for `task`, if the user listed one.
    pub fn get(&self, task: TaskType) -> Option<&[&'static ModelSpec]> {
        self.chains.get(&task).map(Vec::as_slice)
    }
}

/// `"<provider>:<model id>"` to a catalog entry.
fn parse_model_ref(s: &str) -> Result<&'static ModelSpec, String> {
    let (provider, id) = s.split_once(':').ok_or_else(|| format!("`{s}` is not <provider>:<model id>"))?;
    let provider: ProviderId = provider.parse()?;
    find(provider, id.trim()).ok_or_else(|| format!("unknown model `{s}`"))
}
