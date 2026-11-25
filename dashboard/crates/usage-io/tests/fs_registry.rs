//! Private-file helpers and the source registry, against real temp directories.
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

use jev_usage_core::port::SourceId;
use jev_usage_io::clock::FakeClock;
use jev_usage_io::fsutil::{self, FsError};
use jev_usage_io::http::HttpClient;
use jev_usage_io::probe::{FactsCell, LoginFacts};
use jev_usage_io::proc::ProcRunner;
use jev_usage_io::registry::{backfill_job, build, RegistryInputs};
use jev_usage_io::secrets::KeySources;
use jev_usage_io::store::{MetaKey, SqliteStore};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn private_directories_are_created_nested_and_tightened() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b/c");
    fsutil::ensure_private_dir(&nested).unwrap();
    assert_eq!(mode(&nested), 0o700);

    fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();
    fsutil::ensure_private_dir(&nested).unwrap();
    assert_eq!(
        mode(&nested),
        0o700,
        "a looser existing directory is tightened"
    );

    // A path component that is a file cannot become a directory.
    let file = dir.path().join("file");
    fs::write(&file, "x").unwrap();
    assert_eq!(
        fsutil::ensure_private_dir(&file.join("sub")),
        Err(FsError::Invalid)
    );
}

#[test]
fn atomic_private_writes_replace_and_report_what_went_wrong() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("settings.json");
    fsutil::write_private_atomic(&target, b"one").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"one");
    assert_eq!(mode(&target), 0o600);
    fsutil::write_private_atomic(&target, b"two").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"two");
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers.len(),
        1,
        "no temporary file is left behind: {leftovers:?}"
    );

    // A relative name with no directory part writes into the working directory's parent "."
    // only when it can; here an absent parent is reported as missing.
    assert_eq!(
        fsutil::write_private_atomic(&dir.path().join("absent/x.json"), b"x"),
        Err(FsError::Missing)
    );
    // A path with no file name is invalid.
    assert_eq!(
        fsutil::write_private_atomic(std::path::Path::new("/"), b"x"),
        Err(FsError::Invalid)
    );

    // A directory in the way of the rename fails and cleans up its temporary file.
    let blocked = dir.path().join("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("keep"), "x").unwrap();
    assert!(fsutil::write_private_atomic(&blocked, b"x").is_err());
    let names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains(".tmp-")), "{names:?}");

    // A read-only directory is a permission problem (skipped when running as root).
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
    let denied = fsutil::write_private_atomic(&locked.join("x"), b"x");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    if denied.is_ok() {
        return; // root ignores directory permissions
    }
    assert_eq!(denied, Err(FsError::Denied));
}

#[test]
fn removing_a_missing_file_is_fine_and_other_failures_are_classified() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    fs::write(&file, "x").unwrap();
    fsutil::remove_file(&file).unwrap();
    assert!(!file.exists());
    fsutil::remove_file(&file).unwrap();
    // A directory is not a file: the failure is reported, not swallowed.
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    assert!(fsutil::remove_file(&sub).is_err());
}

fn inputs(usage_log: Option<PathBuf>) -> RegistryInputs {
    RegistryInputs {
        usage_log,
        facts: FactsCell::new(LoginFacts::unknown()),
        clock: FakeClock::at(0),
        runner: Arc::new(ProcRunner::system(PathBuf::from("/nonexistent"))),
        http: Arc::new(HttpClient::new().unwrap()),
        keys: Arc::new(|| KeySources {
            env: None,
            env_file: None,
        }),
    }
}

#[test]
fn the_registry_lists_four_real_sources_then_the_three_placeholders() {
    let sources = build(&inputs(None));
    let ids: Vec<SourceId> = sources.iter().map(|s| s.descriptor().id).collect();
    assert_eq!(ids.len(), 7);
    assert_eq!(
        &ids[..4],
        &[
            SourceId::ClaudeGatewayLog,
            SourceId::ClaudeLive,
            SourceId::CodexAppServer,
            SourceId::DeepSeekBalance,
        ]
    );
    let mut unique = ids.clone();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "every source id appears once");
}

#[test]
fn the_backfill_job_runs_once_and_only_with_a_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("history.db")).unwrap());
    assert!(backfill_job(None, Arc::clone(&store)).is_none());

    let log = dir.path().join("usage.jsonl");
    let p = "anthropic-ratelimit-unified-";
    let mut rows = String::new();
    for i in 0..3_u64 {
        use std::fmt::Write as _;
        writeln!(
            rows,
            "{{\"ts_ms\":{},\"ratelimit\":{{\"{p}5h-utilization\":\"0.{i}\",\"{p}5h-reset\":\"9999999\",\"{p}5h-status\":\"allowed\"}}}}",
            1_794_890_000_000_u64 + i * 1_000
        )
        .unwrap();
    }
    fs::write(&log, rows).unwrap();

    let job = backfill_job(Some(log.clone()), Arc::clone(&store)).expect("first run has a job");
    assert_eq!(job.provider, jev_usage_core::provider::ProviderKey::Claude);
    let stored = (job.run)().unwrap();
    assert!(stored >= 1, "the log's rows were imported: {stored}");
    assert_eq!(store.meta_get(MetaKey::BackfillDone).unwrap(), Some(1));
    assert!(
        backfill_job(Some(log), store).is_none(),
        "the import never repeats"
    );
}
