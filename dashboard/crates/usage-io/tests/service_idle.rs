//! The service must sleep when nothing is due. A reading that has gone stale (an old gateway
//! log row) once made the next deadline "now" on every pass, so the loop spun at one core
//! while the app sat hidden (found by `measure-idle.sh`, T12).
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

use jev_usage_core::alerts::Alert;
use jev_usage_core::ipc::HourCycle;
use jev_usage_core::port::{Notifier, UsageSource};
use jev_usage_core::time::UnixMillis;
use jev_usage_io::clock::SystemClock;
use jev_usage_io::http::HttpClient;
use jev_usage_io::probe::{CliProbe, FactsCell, LoginFacts};
use jev_usage_io::proc::ProcRunner;
use jev_usage_io::registry::{self, RegistryInputs};
use jev_usage_io::secrets::KeySources;
use jev_usage_io::service::{self, ServiceDeps, SystemJitter};
use jev_usage_io::settings_file::JsonSettingsStore;
use jev_usage_io::store::SqliteStore;
use jev_usage_io::timer::{SystemTimer, TimerFuture, WallTimer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Silent;
impl Notifier for Silent {
    fn notify(&self, _alert: &Alert, _hour: HourCycle) {}
}

/// The real timer that counts how often the service asks it to sleep.
struct CountingTimer {
    inner: SystemTimer,
    calls: Arc<AtomicU64>,
}

impl WallTimer for CountingTimer {
    fn sleep_until(&self, deadline: UnixMillis) -> TimerFuture<'_> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inner.sleep_until(deadline)
    }
}

/// A runtime built by hand so it can be shut down with a time limit: the real timer waits on
/// a blocking thread, which `#[tokio::test]` would wait for at the end of the test.
#[test]
fn a_stale_reading_does_not_make_the_service_spin() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(stale_reading_service_sleeps());
    runtime.shutdown_timeout(Duration::from_secs(1));
}

async fn stale_reading_service_sleeps() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    // The fixture's rows are days old: its reading is stale from the first pass.
    let log = state.join("usage.jsonl");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/claude-gateway/canary.jsonl"),
        &log,
    )
    .unwrap();
    let missing = root.path().join("none");
    let runner = Arc::new(ProcRunner::with_programs(
        root.path().to_owned(),
        missing.join("codex"),
        missing.join("claude"),
    ));
    let clock = Arc::new(SystemClock);
    let facts = FactsCell::new(LoginFacts::unknown());
    let sources: Vec<Arc<dyn UsageSource>> = registry::build(&RegistryInputs {
        usage_log: Some(log),
        facts: Arc::clone(&facts),
        clock: clock.clone(),
        runner: Arc::clone(&runner),
        http: Arc::new(HttpClient::new().unwrap()),
        keys: Arc::new(|| KeySources {
            env: None,
            env_file: None,
        }),
    });
    let calls = Arc::new(AtomicU64::new(0));
    let (_signals_tx, signals) = tokio::sync::mpsc::channel(8);
    let handle = service::spawn(
        ServiceDeps {
            clock,
            timer: Arc::new(CountingTimer {
                inner: SystemTimer::new(),
                calls: Arc::clone(&calls),
            }),
            jitter: Arc::new(SystemJitter),
            sources,
            probe: Arc::new(CliProbe::new(runner)),
            facts,
            store: Arc::new(SqliteStore::open(&state.join("dashboard/history.db")).unwrap()),
            settings: Arc::new(JsonSettingsStore::new(
                state.join("dashboard/settings.json"),
            )),
            notifier: Arc::new(Silent),
            signals,
            backfill: None,
        },
        &tokio::runtime::Handle::current(),
    );

    // Wait for the stale reading to be in, then count timer sleeps over a quiet window.
    let mut updates = handle.subscribe();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let json = serde_json::to_string(&handle.snapshot()).unwrap();
        if json.contains("\"used_bp\"") {
            break;
        }
        let _ = tokio::time::timeout(Duration::from_millis(300), updates.changed()).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = calls.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let slept = calls.load(Ordering::Relaxed) - before;
    handle.shutdown().await;
    assert!(
        slept < 20,
        "the service asked the timer to sleep {slept} times in 2 s: it is spinning"
    );
}
