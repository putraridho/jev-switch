//! The log tail reader, the file watch and the gateway-log source, against real temp files.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_usage_core::alerts::AlertMemory;
use jev_usage_core::history::{SeriesData, StoreStats};
use jev_usage_core::meter::{Meter, MeterId, Reading, WindowKind};
use jev_usage_core::port::{HistoryStore, SourceError, SourceId, StoreError, UsageSource};
use jev_usage_core::provider::ProviderKey;
use jev_usage_core::time::UnixSeconds;
use jev_usage_core::usage::{Setup, Unavailable};
use jev_usage_io::clock::FakeClock;
use jev_usage_io::probe::{FactsCell, LoginFacts};
use jev_usage_io::sources::claude_log::{backfill, ClaudeGatewayLogSource};
use jev_usage_io::tail::{for_each_line, TailEvent, TailReader, BLOCK, MAX_LINE};
use jev_usage_io::watch::{watch_file, WatchOptions};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

fn append(path: &Path, text: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn collect(reader: &mut TailReader) -> (TailEvent, Vec<String>) {
    let mut lines = Vec::new();
    let event = reader
        .read_new(&mut |line| lines.push(String::from_utf8_lossy(line).into_owned()))
        .unwrap();
    (event, lines)
}

// ---------- tail ----------

#[test]
fn tail_reads_appended_lines_and_carries_partial_ones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    let mut reader = TailReader::new(path.clone());
    assert_eq!(collect(&mut reader), (TailEvent::Missing, vec![]));

    append(&path, "one\ntwo\n");
    assert_eq!(
        collect(&mut reader),
        (
            TailEvent::Read { lines: 2 },
            vec!["one".to_owned(), "two".to_owned()]
        )
    );
    assert_eq!(collect(&mut reader), (TailEvent::Unchanged, vec![]));

    append(&path, "par");
    assert_eq!(collect(&mut reader), (TailEvent::Unchanged, vec![]));
    append(&path, "tial\n");
    assert_eq!(
        collect(&mut reader),
        (TailEvent::Read { lines: 1 }, vec!["partial".to_owned()])
    );
}

#[test]
fn tail_follows_a_rotation_and_a_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    append(&path, "a\n");
    let mut reader = TailReader::new(path.clone());
    assert_eq!(collect(&mut reader).0, TailEvent::Read { lines: 1 });

    // A line lands in the old file, then it is renamed and a new file starts.
    append(&path, "b\n");
    fs::rename(&path, dir.path().join("usage.jsonl.1")).unwrap();
    append(&path, "c\n");
    let (event, lines) = collect(&mut reader);
    assert_eq!(event, TailEvent::Rotated { lines: 2 });
    assert_eq!(
        lines,
        ["b", "c"],
        "the old file is drained before the new one"
    );
    assert_eq!(collect(&mut reader), (TailEvent::Unchanged, vec![]));

    // The same file cut shorter than the offset counts as a rotation too.
    fs::write(&path, "").unwrap();
    assert_eq!(
        collect(&mut reader),
        (TailEvent::Rotated { lines: 0 }, vec![])
    );
    append(&path, "d\n");
    assert_eq!(
        collect(&mut reader),
        (TailEvent::Read { lines: 1 }, vec!["d".to_owned()])
    );

    // The file disappearing is reported as a rotation to nothing, then as missing once a
    // reader that has lost its file keeps asking.
    fs::remove_file(&path).unwrap();
    let (event, _) = collect(&mut reader);
    assert!(matches!(event, TailEvent::Rotated { .. }));
    assert_eq!(collect(&mut reader).0, TailEvent::Missing);
}

#[test]
fn tail_skips_lines_longer_than_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    let mut huge = "x".repeat(MAX_LINE + 10);
    huge.push('\n');
    append(&path, &huge);
    append(&path, "ok\n");
    let mut reader = TailReader::new(path);
    let (event, lines) = collect(&mut reader);
    assert_eq!(lines, ["ok"]);
    assert_eq!(event, TailEvent::Read { lines: 1 });
}

fn accepts_good(line: &[u8]) -> bool {
    line.starts_with(b"good")
}

#[test]
fn seek_newest_finds_the_last_matching_line_and_positions_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    fs::write(&path, "good1\nbad\nbad\n").unwrap();
    let mut reader = TailReader::new(path.clone());
    assert!(reader.seek_newest(1 << 20, &mut accepts_good).unwrap());
    assert_eq!(
        collect(&mut reader),
        (TailEvent::Unchanged, vec![]),
        "history is not replayed"
    );
    append(&path, "new\n");
    assert_eq!(collect(&mut reader).1, ["new"]);

    // Too small a budget never reaches the match.
    let mut reader = TailReader::new(path.clone());
    assert!(!reader.seek_newest(4, &mut accepts_good).unwrap());

    // The previous (rotated) file is searched when the current one has no match.
    let rotated = dir.path().join("rot.jsonl");
    fs::write(dir.path().join("rot.jsonl.1"), "good-old\nbad\n").unwrap();
    fs::write(&rotated, "bad\nbad\n").unwrap();
    let mut reader = TailReader::new(rotated);
    assert!(reader.seek_newest(1 << 20, &mut accepts_good).unwrap());

    // A file that does not exist is simply "not found", and reading later works.
    let late = dir.path().join("late.jsonl");
    let mut reader = TailReader::new(late.clone());
    assert!(!reader.seek_newest(1 << 20, &mut accepts_good).unwrap());
    assert_eq!(collect(&mut reader).0, TailEvent::Missing);
    append(&late, "later\n");
    assert_eq!(collect(&mut reader).1, ["later"]);
}

#[test]
fn seek_newest_reads_back_across_several_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    let filler = format!("bad{}\n", "y".repeat(100_000));
    let mut text = String::from("good-first\n");
    for _ in 0..8 {
        text.push_str(&filler);
    }
    assert!(text.len() > 2 * BLOCK);
    fs::write(&path, &text).unwrap();
    let mut reader = TailReader::new(path);
    assert!(reader.seek_newest(u64::MAX, &mut accepts_good).unwrap());
}

#[test]
fn tail_io_errors_are_reported_as_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    // A directory opens but cannot be read.
    let mut reader = TailReader::new(dir.path().to_path_buf());
    assert_eq!(
        reader.seek_newest(1 << 20, &mut accepts_good),
        Err(Unavailable::Io)
    );
    let mut reader = TailReader::new(dir.path().to_path_buf());
    assert_eq!(reader.read_new(&mut |_| ()), Err(Unavailable::Io));
    assert_eq!(
        for_each_line(&dir.path().join("missing"), &mut |_| ()),
        Err(Unavailable::Io)
    );
}

#[test]
fn for_each_line_counts_complete_lines_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f");
    fs::write(&path, "a\nb\nunfinished").unwrap();
    let mut seen = Vec::new();
    let count = for_each_line(&path, &mut |l| seen.push(l.to_vec())).unwrap();
    assert_eq!(count, 2);
    assert_eq!(seen, [b"a".to_vec(), b"b".to_vec()]);
}

// ---------- watch ----------

fn fast() -> WatchOptions {
    WatchOptions {
        coalesce: Duration::from_millis(30),
        rearm_first: Duration::from_millis(50),
        rearm_max: Duration::from_millis(200),
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

async fn next_signal(
    rx: &mut tokio::sync::mpsc::Receiver<SourceId>,
    within: Duration,
) -> Option<SourceId> {
    tokio::time::timeout(within, rx.recv()).await.ok().flatten()
}

#[test]
fn watch_signals_on_modification_and_recreation_but_not_on_strangers() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    fs::write(&path, "x\n").unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let guard = watch_file(&path, SourceId::ClaudeGatewayLog, tx, fast(), rt.handle());
    // Give the blocking thread a moment to arm the watches.
    std::thread::sleep(Duration::from_millis(300));

    // A different file in the same directory is not the log.
    fs::write(dir.path().join("other.jsonl"), "y\n").unwrap();
    assert_eq!(
        rt.block_on(next_signal(&mut rx, Duration::from_millis(400))),
        None
    );

    // Modifying the log signals once the burst settles.
    append(&path, "more\n");
    assert_eq!(
        rt.block_on(next_signal(&mut rx, Duration::from_secs(5))),
        Some(SourceId::ClaudeGatewayLog)
    );

    // Rotating it away and creating a new one signals through the directory watch.
    while rx.try_recv().is_ok() {}
    fs::rename(&path, dir.path().join("usage.jsonl.1")).unwrap();
    fs::write(&path, "fresh\n").unwrap();
    assert_eq!(
        rt.block_on(next_signal(&mut rx, Duration::from_secs(5))),
        Some(SourceId::ClaudeGatewayLog)
    );

    drop(guard);
    // Wake the blocking thread so it sees the stop flag, then discard the runtime.
    append(&path, "wake\n");
    rt.shutdown_background();
}

#[test]
fn watch_of_a_file_that_does_not_exist_yet_signals_when_it_appears() {
    let rt = runtime();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let guard = watch_file(&path, SourceId::ClaudeGatewayLog, tx, fast(), rt.handle());
    std::thread::sleep(Duration::from_millis(300));
    fs::write(&path, "hello\n").unwrap();
    assert_eq!(
        rt.block_on(next_signal(&mut rx, Duration::from_secs(5))),
        Some(SourceId::ClaudeGatewayLog)
    );
    // With the receiver gone the watcher ends on its own instead of spinning.
    drop(rx);
    append(&path, "again\n");
    std::thread::sleep(Duration::from_millis(300));
    drop(guard);
    append(&path, "wake\n");
    rt.shutdown_background();
}

// ---------- gateway-log source ----------

const T: u64 = 1_794_890_000_000;
const P: &str = "anthropic-ratelimit-unified-";

fn row(ts_ms: u64, windows: &[(&str, &str, u64)]) -> String {
    let headers: Vec<String> = windows
        .iter()
        .flat_map(|(name, used, reset)| {
            [
                format!(r#""{P}{name}-utilization":"{used}""#),
                format!(r#""{P}{name}-reset":"{reset}""#),
                format!(r#""{P}{name}-status":"allowed""#),
            ]
        })
        .collect();
    format!(
        "{{\"ts_ms\":{ts_ms},\"ratelimit\":{{{}}}}}\n",
        headers.join(",")
    )
}

fn used_of(reading: &Reading, kind: &WindowKind) -> Option<u16> {
    reading.meters.as_slice().iter().find_map(|m| match m {
        Meter::Window(w) if &w.kind == kind => Some(w.used.basis_points()),
        _ => None,
    })
}

#[tokio::test]
async fn the_log_source_merges_windows_across_rows_and_drops_stale_ones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    let reset = T / 1000 + 3_600;
    append(
        &path,
        &row(T, &[("5h", "0.4", reset), ("7d", "0.2", reset + 86_400)]),
    );
    let clock = FakeClock::at(T + 5_000);
    let facts = FactsCell::new(LoginFacts::unknown());
    let source = ClaudeGatewayLogSource::new(Some(path.clone()), clock.clone(), facts);

    let first = source.fetch().await.unwrap();
    assert_eq!(first.observed_at.0, T);
    assert_eq!(used_of(&first, &WindowKind::FiveHour), Some(4_000));
    assert_eq!(used_of(&first, &WindowKind::SevenDay), Some(2_000));

    // A newer row updates only the window it carries; the 7-day one is still in the future.
    append(&path, &row(T + 1_000, &[("5h", "0.6", reset)]));
    let second = source.fetch().await.unwrap();
    assert_eq!(second.observed_at.0, T + 1_000);
    assert_eq!(used_of(&second, &WindowKind::FiveHour), Some(6_000));
    assert_eq!(used_of(&second, &WindowKind::SevenDay), Some(2_000));
    assert_eq!(
        second.meters.as_slice()[0].id(),
        MeterId::Window(WindowKind::FiveHour)
    );

    // An older row never overwrites a newer one.
    append(&path, &row(T - 60_000, &[("5h", "0.1", reset)]));
    let third = source.fetch().await.unwrap();
    assert_eq!(used_of(&third, &WindowKind::FiveHour), Some(6_000));

    // Long after the 7-day reading's reset it is dropped; only the newest row's window stays.
    clock.set(T + 10 * 86_400_000);
    append(&path, &row(T + 2_000, &[("5h", "0.7", reset)]));
    let fourth = source.fetch().await.unwrap();
    assert_eq!(used_of(&fourth, &WindowKind::SevenDay), None);
    assert_eq!(used_of(&fourth, &WindowKind::FiveHour), Some(7_000));
}

#[tokio::test]
async fn the_log_source_says_so_when_there_is_nothing_to_read() {
    let clock = FakeClock::at(T);
    let facts = FactsCell::new(LoginFacts::unknown());
    let unconfigured = ClaudeGatewayLogSource::new(None, clock.clone(), facts.clone());
    assert_eq!(
        unconfigured.fetch().await.unwrap_err(),
        SourceError::NotConfigured(Setup::UseGateway)
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    fs::write(&path, "{\"ts_ms\":1}\nnot json\n").unwrap();
    let empty = ClaudeGatewayLogSource::new(Some(path), clock.clone(), facts.clone());
    assert_eq!(
        empty.fetch().await.unwrap_err(),
        SourceError::NotConfigured(Setup::UseGateway)
    );

    // A path that cannot be read at all is an I/O failure, not "use the gateway".
    let dir2 = tempfile::tempdir().unwrap();
    let unreadable = ClaudeGatewayLogSource::new(Some(dir2.path().to_path_buf()), clock, facts);
    assert_eq!(
        unreadable.fetch().await.unwrap_err(),
        SourceError::Unavailable(Unavailable::Io)
    );
}

// ---------- backfill ----------

#[derive(Default)]
struct CountingStore {
    batches: Mutex<Vec<usize>>,
    fail_after: Option<usize>,
}

impl HistoryStore for CountingStore {
    fn record(&self, _: ProviderKey, _: &Reading) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn record_all(&self, provider: ProviderKey, readings: &[Reading]) -> Result<u64, StoreError> {
        assert_eq!(provider, ProviderKey::Claude);
        let mut batches = self.batches.lock().unwrap();
        if self.fail_after.is_some_and(|limit| batches.len() >= limit) {
            return Err(StoreError::Corrupt);
        }
        batches.push(readings.len());
        Ok(u64::try_from(readings.len()).unwrap())
    }
    fn series(
        &self,
        _: ProviderKey,
        _: &MeterId,
        _: UnixSeconds,
        _: UnixSeconds,
        _: u16,
    ) -> Result<SeriesData, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn load_alerts(&self) -> Result<AlertMemory, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn save_alerts(&self, _: &AlertMemory) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn prune(&self, _: UnixSeconds) -> Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn stats(&self) -> Result<StoreStats, StoreError> {
        Err(StoreError::Unavailable)
    }
}

fn rows(count: usize, base: u64) -> String {
    (0..count)
        .map(|i| {
            row(
                base + u64::try_from(i).unwrap(),
                &[("5h", "0.5", 9_999_999)],
            )
        })
        .collect()
}

#[test]
fn backfill_reads_the_rotated_file_first_and_batches_by_512() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    // 600 rows in the rotated file, 20 in the current one, plus noise that is not a sample;
    // the batch carries over from one file to the next.
    fs::write(dir.path().join("usage.jsonl.1"), rows(600, T)).unwrap();
    fs::write(
        &path,
        format!("{}garbage\n{}", rows(10, T + 10_000), rows(10, T + 20_000)),
    )
    .unwrap();
    let store = CountingStore::default();
    assert_eq!(backfill(&path, &store).unwrap(), 620);
    assert_eq!(*store.batches.lock().unwrap(), [512, 108]);

    // Nothing to read is not an error.
    let none = CountingStore::default();
    assert_eq!(
        backfill(&dir.path().join("absent.jsonl"), &none).unwrap(),
        0
    );
    assert!(none.batches.lock().unwrap().is_empty());
}

#[test]
fn backfill_reports_store_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.jsonl");
    // A failure in a full batch ends the run at the end of that file.
    fs::write(&path, rows(600, T)).unwrap();
    let failing = CountingStore {
        fail_after: Some(0),
        ..CountingStore::default()
    };
    assert_eq!(backfill(&path, &failing), Err(StoreError::Corrupt));
    // A failure on the final partial batch is reported too.
    fs::write(&path, rows(5, T)).unwrap();
    let failing = CountingStore {
        fail_after: Some(0),
        ..CountingStore::default()
    };
    assert_eq!(backfill(&path, &failing), Err(StoreError::Corrupt));
    // A file that exists but cannot be read maps to the store being unavailable.
    let unreadable_dir = dir.path().join("a_directory");
    fs::create_dir(&unreadable_dir).unwrap();
    assert_eq!(
        backfill(&unreadable_dir, &CountingStore::default()),
        Err(StoreError::Unavailable)
    );
}
