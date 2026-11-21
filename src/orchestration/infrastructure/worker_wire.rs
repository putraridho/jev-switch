//! The worker registry's on-disk representation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::orchestration::domain::worker::{Owner, Saved};

#[derive(Serialize, Deserialize)]
struct OwnerWire {
    pid: u32,
    #[serde(default)]
    start: Option<u64>,
}

impl From<Owner> for OwnerWire {
    fn from(value: Owner) -> Self {
        Self { pid: value.pid, start: value.start }
    }
}

impl From<OwnerWire> for Owner {
    fn from(value: OwnerWire) -> Self {
        Self { pid: value.pid, start: value.start }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SavedWire {
    worker_id: String,
    path: PathBuf,
    repo: PathBuf,
    provider: String,
    model: String,
    created_ms: u64,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    shown: BTreeSet<String>,
    #[serde(default)]
    covered: BTreeMap<String, usize>,
    #[serde(default)]
    merged: BTreeSet<String>,
    #[serde(default)]
    owner: Option<OwnerWire>,
}

impl From<&Saved> for SavedWire {
    fn from(value: &Saved) -> Self {
        Self {
            worker_id: value.worker_id.clone(),
            path: value.path.clone(),
            repo: value.repo.clone(),
            provider: value.provider.clone(),
            model: value.model.clone(),
            created_ms: value.created_ms,
            running: value.running,
            shown: value.shown.clone(),
            covered: value.covered.clone(),
            merged: value.merged.clone(),
            owner: value.owner.clone().map(Into::into),
        }
    }
}

impl From<SavedWire> for Saved {
    fn from(value: SavedWire) -> Self {
        Self {
            worker_id: value.worker_id,
            path: value.path,
            repo: value.repo,
            provider: value.provider,
            model: value.model,
            created_ms: value.created_ms,
            running: value.running,
            shown: value.shown,
            covered: value.covered,
            merged: value.merged,
            owner: value.owner.map(Into::into),
        }
    }
}
