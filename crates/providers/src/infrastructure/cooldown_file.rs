//! Shared cooldown file format and atomic persistence.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::domain::reset_time::cooldowns::Cooldowns;
use jev_shared::ProviderId;

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct CooldownFile(Value);

impl From<&Cooldowns> for CooldownFile {
    fn from(cooldowns: &Cooldowns) -> Self {
        Self(Value::Object(cooldowns.entries().iter().map(|(provider, until)| (provider.as_str().to_string(), Value::from(*until))).collect::<Map<_, _>>()))
    }
}

impl From<CooldownFile> for Cooldowns {
    fn from(file: CooldownFile) -> Self {
        file.0.as_object().into_iter().flatten()
            .filter_map(|(name, value)| Some((name.parse::<ProviderId>().ok()?, value.as_u64()?)))
            .collect::<std::collections::HashMap<_, _>>().into()
    }
}

pub fn load(path: &Path) -> Cooldowns {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<CooldownFile>(&text).ok()).map(Cooldowns::from).unwrap_or_default()
}

pub fn save(path: &Path, cooldowns: &Cooldowns, now_ms: u64) -> std::io::Result<Cooldowns> {
    let merged = cooldowns.merged(&load(path), now_ms);
    super::atomic_file::write_atomic(path, &serde_json::to_string(&CooldownFile::from(&merged))?)?;
    Ok(merged)
}

pub fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

#[cfg(test)]
#[path = "cooldown_file_tests.rs"]
mod tests;
