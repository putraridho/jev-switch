//! Which providers are cooling down and until when, kept in memory and in a shared
//! JSON file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value};

use super::resolver::system_date;
use super::{cooldown_ms_with, Resolver};
use crate::shared::ProviderId;

/// When each provider's usage limit resets (epoch ms).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cooldowns {
    pub(super) until_ms: HashMap<ProviderId, u64>,
}

impl Cooldowns {
    /// Start (or restart) a cooldown after a quota error.
    pub fn start(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64) {
        self.start_with(p, resets_at, now_ms, &system_date);
    }

    /// [`Cooldowns::start`] with an injected wall-clock resolver.
    pub fn start_with(&mut self, p: ProviderId, resets_at: Option<&str>, now_ms: u64, resolve: Resolver) {
        self.until_ms.insert(p, now_ms.saturating_add(cooldown_ms_with(resets_at, now_ms, resolve)));
    }

    /// Milliseconds left if the provider is cooling down, else `None`.
    pub fn active(&self, p: ProviderId, now_ms: u64) -> Option<u64> {
        self.until_ms.get(&p).and_then(|u| u.checked_sub(now_ms)).filter(|left| *left > 0)
    }

    /// Forget a cooldown in this copy only. It does NOT lift a saved cooldown: once a
    /// cooldown has been saved it cannot be forgiven, it can only expire. That is on
    /// purpose. [`Cooldowns::save`] merges the file in (later expiry per provider
    /// wins) and [`crate::providers::application::provider_registry::Runtime`] unions the file into what it reports,
    /// because other processes share the file and one process must not erase a limit
    /// another just learned. So after `clear` the entry comes back from disk on the
    /// next save or read. Nothing in production calls this; it exists for tests and
    /// for dropping a throwaway copy.
    pub fn clear(&mut self, p: ProviderId) {
        self.until_ms.remove(&p);
    }

    /// Per provider, the later of the two expiries (expired entries kept).
    pub fn union(&self, other: &Self) -> Self {
        let mut until_ms = self.until_ms.clone();
        for (p, u) in &other.until_ms {
            let e = until_ms.entry(*p).or_insert(*u);
            *e = (*e).max(*u);
        }
        Self { until_ms }
    }

    /// The later expiry per provider across both sets, minus entries already over.
    pub fn merged(&self, other: &Self, now_ms: u64) -> Self {
        let mut m = self.union(other);
        m.until_ms.retain(|_, u| *u > now_ms);
        m
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

    /// Merge into the file and return what was written. Read-modify-write: other
    /// processes (`jev-code`, every `delegate_mcp`) share the file, so what is on disk
    /// is merged in (later expiry per provider wins) and entries already over at
    /// `now_ms` are dropped. Written atomically (temp file in the same directory, then
    /// rename) so a crash mid-write never leaves a half-written file. Writers that do
    /// not overlap in time can never lose each other's updates; a simultaneous pair
    /// can, within the few microseconds between the read and the rename.
    pub fn save(&self, path: &Path, now_ms: u64) -> std::io::Result<Self> {
        let merged = self.merged(&Self::load(path), now_ms);
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, merged.to_json().to_string())?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(merged)
    }
}

/// Cheap fingerprint of the cooldown file: modification time and length.
type FileStamp = (SystemTime, u64);

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The last copy of the cooldown file this process read, reloaded only when the file
/// changed (another process saved), so polling it costs one `stat`.
#[derive(Debug, Default)]
pub struct DiskWatch {
    stamp: Option<FileStamp>,
    disk: Cooldowns,
    reloads: usize,
}

impl DiskWatch {
    /// The file's current contents, re-read only if its stamp changed since the last
    /// call. A missing or garbled file is an empty set.
    pub fn refresh(&mut self, path: &Path) -> &Cooldowns {
        let stamp = file_stamp(path);
        if stamp != self.stamp {
            self.disk = Cooldowns::load(path);
            self.stamp = stamp;
            self.reloads += 1;
        }
        &self.disk
    }

    /// How many times the file was actually read.
    pub fn reloads(&self) -> usize {
        self.reloads
    }
}
