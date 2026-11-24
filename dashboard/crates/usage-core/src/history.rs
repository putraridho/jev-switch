//! History rules: what is stored and how a series is shaped (DESIGN 4.5, 7.5). The types
//! and constants are frozen; T02 writes the function bodies.

use crate::ipc::HistoryRange;
use crate::meter::WindowKind;
use crate::time::UnixSeconds;
use std::time::Duration;

/// How long raw samples are kept: 45 days.
pub const RAW_RETENTION: Duration = Duration::from_hours(45 * 24);
/// How long hourly rollups are kept: 400 days.
pub const ROLLUP_RETENTION: Duration = Duration::from_hours(400 * 24);

/// One stored sample of one meter.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct StoredSample {
    /// When it was observed.
    pub at: UnixSeconds,
    /// Basis points for a window, micros for a balance.
    pub value: i64,
    /// The window's reset time at that moment, if known.
    pub resets_at: Option<UnixSeconds>,
}

/// One point of a series: the last value of its bucket and the bucket's extremes.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct SamplePoint {
    /// The bucket's time.
    pub at: UnixSeconds,
    /// The last value in the bucket.
    pub value: i64,
    /// The lowest value in the bucket.
    pub lo: i64,
    /// The highest value in the bucket.
    pub hi: i64,
}

/// What the store returns for a range.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct SeriesData {
    /// The points, oldest first.
    pub points: Vec<SamplePoint>,
    /// Window ends inside the range, at most 64; empty for a rollup query.
    pub resets: Vec<UnixSeconds>,
    /// The width of one bucket.
    pub bucket: Duration,
}

/// The size of the store, for Diagnostics.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct StoreStats {
    /// The database file's size in bytes.
    pub bytes: u64,
    /// The number of raw samples.
    pub samples: u64,
}

/// The write rule (DESIGN 7.5): a candidate not newer than `last` is dropped; otherwise it
/// is stored when the value or reset time differs, or 15 minutes have passed.
pub fn should_store(last: Option<&StoredSample>, candidate: &StoredSample) -> bool {
    let _ = (last, candidate);
    false
}

/// At most `max` points, keeping the first, the last and each bucket's extremes.
pub fn downsample(points: Vec<SamplePoint>, max: u16) -> Vec<SamplePoint> {
    let _ = (points, max);
    Vec::new()
}

/// Collapses a sliding `resets_at` into one mark per window instance (at most 64).
pub fn reset_marks(sorted_resets: &[UnixSeconds], kind: &WindowKind) -> Vec<UnixSeconds> {
    let _ = (sorted_resets, kind);
    Vec::new()
}

/// The longest hole the chart bridges: `max(75 min, 2.5 x bucket)`.
pub fn gap_for(bucket: Duration) -> Duration {
    let _ = bucket;
    Duration::ZERO
}

/// The `(from, to)` of a range that ends at `now`.
pub fn range_bounds(range: HistoryRange, now: UnixSeconds) -> (UnixSeconds, UnixSeconds) {
    let _ = range;
    (now, now)
}
