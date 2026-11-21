//! What is installed, each with its checksum state.

use std::path::{Path, PathBuf};

use super::manifest::read_manifest;
use super::pristine::changed_files;
use super::{extensions_dir, load_registry, Record, MANIFEST};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Every file still has the checksum it was installed with.
    Verified,
    /// These files were changed, or are gone, since installing.
    Changed(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub version: String,
    pub source: PathBuf,
    pub state: State,
}

pub(super) fn listing_of(user: &Path, record: &Record) -> Listing {
    let dir = extensions_dir(user).join(&record.name);
    let state = match read_manifest(&dir) {
        Err(_) => State::Changed(vec![MANIFEST.to_string()]),
        Ok(manifest) => {
            let bad = changed_files(&dir, &manifest);
            if bad.is_empty() {
                State::Verified
            } else {
                State::Changed(bad)
            }
        }
    };
    Listing { name: record.name.clone(), version: record.version.clone(), source: record.source.clone(), state }
}

/// What is installed, each with its checksum state.
pub fn list(user: &Path) -> Vec<Listing> {
    load_registry(user).extensions.iter().map(|r| listing_of(user, r)).collect()
}
