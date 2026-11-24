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
    match last {
        None => true,
        Some(last) => {
            candidate.at.0 > last.at.0
                && (candidate.value != last.value
                    || candidate.resets_at != last.resets_at
                    || candidate.at.0.saturating_sub(last.at.0) >= 900)
        }
    }
}

/// At most `max` points, keeping the first, the last and each bucket's extremes.
pub fn downsample(points: Vec<SamplePoint>, max: u16) -> Vec<SamplePoint> {
    let limit = usize::from(max);
    if points.len() <= limit {
        return points;
    }
    if limit == 0 {
        return Vec::new();
    }
    if limit == 1 {
        return points.into_iter().take(1).collect();
    }
    let mut result = Vec::new();
    if let Some(first) = points.first() {
        result.push(*first);
    }
    let inner = points.len().saturating_sub(2);
    let buckets = limit.saturating_sub(2);
    if buckets > 0 {
        for bucket in 0..buckets {
            let start = 1_usize.saturating_add(
                inner
                    .saturating_mul(bucket)
                    .checked_div(buckets)
                    .unwrap_or(0),
            );
            let end = 1_usize.saturating_add(
                inner
                    .saturating_mul(bucket.saturating_add(1))
                    .checked_div(buckets)
                    .unwrap_or(0),
            );
            if let Some(slice) = points.get(start..end) {
                if let Some(last) = slice.last() {
                    let lo = slice.iter().map(|p| p.lo).min().unwrap_or(last.lo);
                    let hi = slice.iter().map(|p| p.hi).max().unwrap_or(last.hi);
                    result.push(SamplePoint { lo, hi, ..*last });
                }
            }
        }
    }
    if let Some(last) = points.last() {
        result.push(*last);
    }
    result
}

/// Collapses a sliding `resets_at` into one mark per window instance (at most 64).
pub fn reset_marks(sorted_resets: &[UnixSeconds], kind: &WindowKind) -> Vec<UnixSeconds> {
    let minutes = match kind {
        WindowKind::FiveHour => 300,
        WindowKind::SevenDay => 10_080,
        WindowKind::Monthly => 43_200,
        WindowKind::Rolling { minutes } => u64::from(*minutes),
        WindowKind::Named { .. } => 120,
    };
    let distance = minutes.saturating_mul(30).max(1);
    let mut marks: Vec<UnixSeconds> = Vec::new();
    for &at in sorted_resets {
        if marks
            .last()
            .is_none_or(|last| at.0.saturating_sub(last.0) >= distance)
        {
            marks.push(at);
            if marks.len() > 64 {
                return Vec::new();
            }
        }
    }
    marks
}

/// The longest hole the chart bridges: `max(75 min, 2.5 x bucket)`.
pub fn gap_for(bucket: Duration) -> Duration {
    Duration::from_mins(75).max(
        bucket
            .saturating_mul(2)
            .saturating_add(bucket.checked_div(2).unwrap_or(Duration::ZERO)),
    )
}

/// The `(from, to)` of a range that ends at `now`.
pub fn range_bounds(range: HistoryRange, now: UnixSeconds) -> (UnixSeconds, UnixSeconds) {
    let days: u64 = match range {
        HistoryRange::Day => 1,
        HistoryRange::Week => 7,
        HistoryRange::Month => 30,
        HistoryRange::Quarter => 90,
    };
    (
        UnixSeconds(now.0.saturating_sub(days.saturating_mul(86_400))),
        now,
    )
}
