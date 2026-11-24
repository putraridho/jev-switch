//! SQLite history storage. All methods do file IO; callers use `spawn_blocking`.
use jev_usage_core::alerts::{AlertEntry, AlertLevel, AlertMemory};
use jev_usage_core::history::{self, SamplePoint, SeriesData, StoreStats, StoredSample};
use jev_usage_core::meter::{Meter, MeterId, Reading, WindowKind};
use jev_usage_core::port::{HistoryStore, StoreError};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::UnixSeconds;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::Duration;

/// The schema version this binary writes.
pub const SCHEMA_VERSION: i64 = 1;
/// The integers kept in the store's `meta` table.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum MetaKey {
    /// First-run backfill finished.
    BackfillDone,
    /// Last prune time.
    LastPruneAt,
    /// Window width.
    WindowWidth,
    /// Window height.
    WindowHeight,
}
impl MetaKey {
    fn key(self) -> &'static str {
        match self {
            Self::BackfillDone => "backfill_done",
            Self::LastPruneAt => "last_prune_at",
            Self::WindowWidth => "window_width",
            Self::WindowHeight => "window_height",
        }
    }
}
/// One connection behind a mutex. Every method does file IO: callers use `spawn_blocking`.
#[derive(Debug)]
pub struct SqliteStore {
    connection: Mutex<Connection>,
    path: PathBuf,
    newer: bool,
    rows: AtomicU64,
}

fn err(e: rusqlite::Error) -> StoreError {
    match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            StoreError::Corrupt
        }
        _ => StoreError::Unavailable,
    }
}
fn number(n: u64) -> Result<i64, StoreError> {
    i64::try_from(n).map_err(|_| StoreError::Unavailable)
}
fn seconds(n: i64) -> Result<UnixSeconds, StoreError> {
    u64::try_from(n)
        .map(UnixSeconds)
        .map_err(|_| StoreError::Corrupt)
}
fn optional_seconds(n: Option<i64>) -> Result<Option<UnixSeconds>, StoreError> {
    n.map(seconds).transpose()
}
fn open_connection(path: &Path) -> Result<(Connection, bool), StoreError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| StoreError::Unavailable)?;
    let mut c = Connection::open(path).map_err(err)?;
    let version: i64 = c
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(err)?;
    if version > SCHEMA_VERSION {
        return Ok((c, true));
    }
    c.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA busy_timeout=2000; PRAGMA temp_store=MEMORY;").map_err(err)?;
    if version == 0 {
        let tx = c.transaction().map_err(err)?;
        tx.execute_batch("CREATE TABLE meter (id INTEGER PRIMARY KEY, provider TEXT NOT NULL, meter_key TEXT NOT NULL, UNIQUE(provider,meter_key));
CREATE TABLE sample (meter_id INTEGER NOT NULL REFERENCES meter(id), at_s INTEGER NOT NULL, value INTEGER NOT NULL, resets_at_s INTEGER, PRIMARY KEY(meter_id,at_s)) WITHOUT ROWID;
CREATE TABLE rollup_hour (meter_id INTEGER NOT NULL REFERENCES meter(id), hour_s INTEGER NOT NULL, lo INTEGER NOT NULL, hi INTEGER NOT NULL, last INTEGER NOT NULL, PRIMARY KEY(meter_id,hour_s)) WITHOUT ROWID;
CREATE TABLE alert_memory (meter_id INTEGER NOT NULL REFERENCES meter(id), level INTEGER NOT NULL, fired_at_s INTEGER NOT NULL, window_resets_at_s INTEGER, PRIMARY KEY(meter_id,level)) WITHOUT ROWID;
CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID; PRAGMA user_version=1;").map_err(err)?;
        tx.commit().map_err(err)?;
    }
    Ok((c, false))
}
fn meter_id(c: &Connection, provider: ProviderKey, meter: &MeterId) -> Result<i64, StoreError> {
    c.execute(
        "INSERT OR IGNORE INTO meter(provider,meter_key) VALUES(?1,?2)",
        params![provider.as_str(), meter.key()],
    )
    .map_err(err)?;
    c.query_row(
        "SELECT id FROM meter WHERE provider=?1 AND meter_key=?2",
        params![provider.as_str(), meter.key()],
        |r| r.get(0),
    )
    .map_err(err)
}
fn record_one(c: &Connection, provider: ProviderKey, reading: &Reading) -> Result<u64, StoreError> {
    let mut count = 0_u64;
    for meter in reading.meters.as_slice() {
        let id = meter_id(c, provider, &meter.id())?;
        let (value, reset) = match meter {
            Meter::Window(w) => (i64::from(w.used.basis_points()), w.resets_at),
            Meter::Balance(b) => (b.total.micros(), None),
        };
        let candidate = StoredSample {
            at: reading.observed_at.seconds(),
            value,
            resets_at: reset,
        };
        let previous: Option<(i64,i64,Option<i64>)> = c.query_row("SELECT at_s,value,resets_at_s FROM sample WHERE meter_id=?1 ORDER BY at_s DESC LIMIT 1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(err)?;
        let last = previous
            .map(|(at, value, reset)| {
                Ok(StoredSample {
                    at: seconds(at)?,
                    value,
                    resets_at: optional_seconds(reset)?,
                })
            })
            .transpose()?;
        if history::should_store(last.as_ref(), &candidate) {
            c.execute(
                "INSERT INTO sample VALUES(?1,?2,?3,?4)",
                params![
                    id,
                    number(candidate.at.0)?,
                    value,
                    reset.map(|v| number(v.0)).transpose()?
                ],
            )
            .map_err(err)?;
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}
impl SqliteStore {
    /// Opens a private database, recovering a corrupt file by moving it aside.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let parent = path.parent().ok_or(StoreError::Unavailable)?;
        crate::fsutil::ensure_private_dir(parent).map_err(|_| StoreError::Unavailable)?;
        let (connection, newer) = match open_connection(path) {
            Err(StoreError::Corrupt) => {
                let mut destination = path.as_os_str().to_os_string();
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| StoreError::Unavailable)?
                    .as_nanos();
                destination.push(format!(".corrupt-{stamp}"));
                std::fs::rename(path, PathBuf::from(destination))
                    .map_err(|_| StoreError::Unavailable)?;
                open_connection(path)?
            }
            other => other?,
        };
        Ok(Self {
            connection: Mutex::new(connection),
            path: path.to_path_buf(),
            newer,
            rows: AtomicU64::new(0),
        })
    }
    /// Reads one integer from the meta table.
    pub fn meta_get(&self, key: MetaKey) -> Result<Option<i64>, StoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?
            .query_row("SELECT value FROM meta WHERE key=?1", [key.key()], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)
    }
    /// Sets one integer in the meta table.
    pub fn meta_set(&self, key: MetaKey, value: i64) -> Result<(), StoreError> {
        if self.newer {
            return Err(StoreError::Unavailable);
        }
        self.connection.lock().map_err(|_| StoreError::Unavailable)?.execute("INSERT INTO meta VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key.key(),value]).map_err(err)?;
        Ok(())
    }
    /// Rows stepped by the last series query.
    #[cfg(feature = "test-support")]
    pub fn last_series_rows(&self) -> u64 {
        self.rows.load(Ordering::Relaxed)
    }
}
impl HistoryStore for SqliteStore {
    fn record(&self, provider: ProviderKey, reading: &Reading) -> Result<(), StoreError> {
        self.record_all(provider, std::slice::from_ref(reading))
            .map(|_| ())
    }
    fn record_all(&self, provider: ProviderKey, readings: &[Reading]) -> Result<u64, StoreError> {
        if self.newer {
            return Err(StoreError::Unavailable);
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let tx = c.transaction().map_err(err)?;
        let mut count = 0_u64;
        for reading in readings {
            count = count.saturating_add(record_one(&tx, provider, reading)?);
        }
        tx.commit().map_err(err)?;
        Ok(count)
    }
    fn series(
        &self,
        provider: ProviderKey,
        meter: &MeterId,
        from: UnixSeconds,
        to: UnixSeconds,
        max_points: u16,
    ) -> Result<SeriesData, StoreError> {
        let span = to.0.saturating_sub(from.0);
        let width = if max_points == 0 {
            1
        } else {
            span.saturating_add(1)
                .div_ceil(u64::from(max_points))
                .max(1)
        };
        let mut result = SeriesData {
            points: Vec::new(),
            resets: Vec::new(),
            bucket: Duration::from_secs(width),
        };
        self.rows.store(0, Ordering::Relaxed);
        if max_points == 0 || from > to {
            return Ok(result);
        }
        let c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let rollup = span > history::RAW_RETENTION.as_secs();
        let sql = "WITH chosen AS (SELECT s.at_s AS t,s.value AS lo,s.value AS hi,s.value AS v FROM sample s JOIN meter m ON m.id=s.meter_id WHERE m.provider=?1 AND m.meter_key=?2 AND s.at_s BETWEEN ?3 AND ?4 UNION ALL SELECT r.hour_s,r.lo,r.hi,r.last FROM rollup_hour r JOIN meter m ON m.id=r.meter_id WHERE ?6 AND m.provider=?1 AND m.meter_key=?2 AND r.hour_s BETWEEN ?3 AND ?4 AND r.hour_s < COALESCE((SELECT MIN(s.at_s) FROM sample s WHERE s.meter_id=r.meter_id),9223372036854775807)), ranked AS (SELECT *, (t-?3)/?5 AS b, ROW_NUMBER() OVER (PARTITION BY (t-?3)/?5 ORDER BY t DESC) AS rn FROM chosen) SELECT MAX(t),MIN(lo),MAX(hi),MAX(CASE WHEN rn=1 THEN v END) FROM ranked GROUP BY b ORDER BY b";
        let mut stmt = c.prepare(sql).map_err(err)?;
        let mut rows = stmt
            .query(params![
                provider.as_str(),
                meter.key(),
                number(from.0)?,
                number(to.0)?,
                number(width)?,
                rollup
            ])
            .map_err(err)?;
        while let Some(row) = rows.next().map_err(err)? {
            result.points.push(SamplePoint {
                at: seconds(row.get(0).map_err(err)?)?,
                lo: row.get(1).map_err(err)?,
                hi: row.get(2).map_err(err)?,
                value: row.get(3).map_err(err)?,
            });
        }
        if !rollup {
            if let MeterId::Window(kind) = meter {
                let distance = match kind {
                    WindowKind::FiveHour => 9000,
                    WindowKind::SevenDay => 302_400,
                    WindowKind::Monthly => 1_296_000,
                    WindowKind::Rolling { minutes } => {
                        i64::from(*minutes).saturating_mul(30).max(1)
                    }
                    WindowKind::Named { .. } => 3600,
                };
                let mut stmt = c.prepare("SELECT MIN(s.resets_at_s) FROM sample s JOIN meter m ON m.id=s.meter_id WHERE m.provider=?1 AND m.meter_key=?2 AND s.at_s BETWEEN ?3 AND ?4 AND s.resets_at_s BETWEEN ?3 AND ?4 GROUP BY s.resets_at_s/?5 ORDER BY 1 LIMIT 130").map_err(err)?;
                let values = stmt
                    .query_map(
                        params![
                            provider.as_str(),
                            meter.key(),
                            number(from.0)?,
                            number(to.0)?,
                            distance
                        ],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(err)?;
                let mut resets = Vec::new();
                for value in values {
                    resets.push(seconds(value.map_err(err)?)?);
                }
                self.rows.store(
                    u64::try_from(resets.len()).unwrap_or(u64::MAX),
                    Ordering::Relaxed,
                );
                result.resets = history::reset_marks(&resets, kind);
            }
        }
        self.rows.fetch_add(
            u64::try_from(result.points.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        Ok(result)
    }
    fn load_alerts(&self) -> Result<AlertMemory, StoreError> {
        let c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let mut stmt = c.prepare("SELECT m.provider,m.meter_key,a.level,a.fired_at_s,a.window_resets_at_s FROM alert_memory a JOIN meter m ON m.id=a.meter_id").map_err(err)?;
        let mut rows = stmt.query([]).map_err(err)?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let p: String = row.get(0).map_err(err)?;
            let key: String = row.get(1).map_err(err)?;
            let provider = ProviderKey::ALL
                .into_iter()
                .find(|p2| p2.as_str() == p)
                .ok_or(StoreError::Corrupt)?;
            let level = match row.get::<_, i64>(2).map_err(err)? {
                1 => AlertLevel::Warn,
                2 => AlertLevel::Critical,
                _ => return Err(StoreError::Corrupt),
            };
            entries.push(AlertEntry {
                provider,
                meter: MeterId::parse_key(&key).ok_or(StoreError::Corrupt)?,
                level,
                fired_at: seconds(row.get(3).map_err(err)?)?,
                window_resets_at: optional_seconds(row.get(4).map_err(err)?)?,
            });
        }
        Ok(AlertMemory { entries })
    }
    fn save_alerts(&self, memory: &AlertMemory) -> Result<(), StoreError> {
        if self.newer {
            return Err(StoreError::Unavailable);
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let tx = c.transaction().map_err(err)?;
        tx.execute("DELETE FROM alert_memory", []).map_err(err)?;
        for entry in &memory.entries {
            let id = meter_id(&tx, entry.provider, &entry.meter)?;
            let level = match entry.level {
                AlertLevel::Warn => 1,
                AlertLevel::Critical => 2,
            };
            tx.execute(
                "INSERT INTO alert_memory VALUES(?1,?2,?3,?4)",
                params![
                    id,
                    level,
                    number(entry.fired_at.0)?,
                    entry.window_resets_at.map(|v| number(v.0)).transpose()?
                ],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }
    fn prune(&self, now: UnixSeconds) -> Result<(), StoreError> {
        if self.newer {
            return Err(StoreError::Unavailable);
        }
        let mut c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let tx = c.transaction().map_err(err)?;
        let cutoff = number(now.0.saturating_sub(history::RAW_RETENTION.as_secs()))?;
        tx.execute("INSERT INTO rollup_hour SELECT meter_id,(at_s/3600)*3600,MIN(value),MAX(value), (SELECT s2.value FROM sample s2 WHERE s2.meter_id=s.meter_id AND s2.at_s/3600=s.at_s/3600 AND s2.at_s<?1 ORDER BY s2.at_s DESC LIMIT 1) FROM sample s WHERE at_s<?1 GROUP BY meter_id,at_s/3600 ON CONFLICT(meter_id,hour_s) DO UPDATE SET lo=MIN(lo,excluded.lo),hi=MAX(hi,excluded.hi),last=excluded.last", [cutoff]).map_err(err)?;
        tx.execute("DELETE FROM sample WHERE at_s<?1", [cutoff])
            .map_err(err)?;
        tx.execute(
            "DELETE FROM rollup_hour WHERE hour_s<?1",
            [number(
                now.0.saturating_sub(history::ROLLUP_RETENTION.as_secs()),
            )?],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        c.execute_batch("PRAGMA incremental_vacuum").map_err(err)
    }
    fn stats(&self) -> Result<StoreStats, StoreError> {
        let c = self
            .connection
            .lock()
            .map_err(|_| StoreError::Unavailable)?;
        let samples: i64 = c
            .query_row("SELECT COUNT(*) FROM sample", [], |r| r.get(0))
            .map_err(err)?;
        Ok(StoreStats {
            bytes: std::fs::metadata(&self.path)
                .map_err(|_| StoreError::Unavailable)?
                .len(),
            samples: u64::try_from(samples).map_err(|_| StoreError::Corrupt)?,
        })
    }
}
