//! End to end (DESIGN 8.10): the real service, the real adapters over stub programs, the real
//! store and settings files in a temp state directory. Canary-laced inputs go in; every
//! emitted `Snapshot`, every file's raw bytes (SQLite WAL included) and the `diag` output
//! are searched for the canaries.
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
use jev_usage_core::ipc::{HourCycle, RefreshTarget};
use jev_usage_core::port::{Notifier, UsageSource};
use jev_usage_io::clock::SystemClock;
use jev_usage_io::diag;
use jev_usage_io::http::HttpClient;
use jev_usage_io::probe::{CliProbe, FactsCell, LoginFacts};
use jev_usage_io::proc::ProcRunner;
use jev_usage_io::registry::{self, RegistryInputs};
use jev_usage_io::secrets::KeySources;
use jev_usage_io::service::{self, ServiceDeps, ShellFacts, SystemJitter};
use jev_usage_io::settings_file::JsonSettingsStore;
use jev_usage_io::store::SqliteStore;
use jev_usage_io::timer::SystemTimer;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CANARIES: [&str; 4] = [
    "JEVCANARY",
    "jevcanary_9f3c1b7e",
    "canary-email",
    "example.invalid",
];

struct Silent;
impl Notifier for Silent {
    fn notify(&self, _alert: &Alert, _hour: HourCycle) {}
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn write_exe(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `codex`: login status says "logged in"; `app-server` first emits a notification that
/// carries the canary e-mail, then the recorded rate-limit response (id 2).
fn codex_stub() -> String {
    let response = fixtures().join("codex-app-server/rate_limits_read.response.json");
    format!(
        "#!/bin/sh\n\
         if [ \"$1\" = login ]; then printf 'Logged in using ChatGPT\\n' >&2; exit 0; fi\n\
         printf '%s\\n' '{{\"method\":\"account/updated\",\"params\":{{\"email\":\"canary-email@example.invalid\",\"name\":\"JEVCANARY_9f3c1b7e_DO_NOT_SURFACE\"}}}}'\n\
         cat '{}'\n\
         cat >/dev/null\n",
        response.display()
    )
}

/// `claude`: `auth status` prints the canary-laced JSON of the fixture.
fn claude_stub() -> String {
    let status = fixtures().join("claude-auth/logged_in.json");
    format!("#!/bin/sh\ncat '{}'\n", status.display())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canaries_never_reach_ipc_store_settings_or_diag() {
    let capture = diag::capture();
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let bin = root.path().join("bin");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    write_exe(&bin.join("codex"), &codex_stub());
    write_exe(&bin.join("claude"), &claude_stub());
    let log = state.join("usage.jsonl");
    std::fs::copy(fixtures().join("claude-gateway/canary.jsonl"), &log).unwrap();

    let runner = Arc::new(ProcRunner::with_programs(
        root.path().to_owned(),
        bin.join("codex"),
        bin.join("claude"),
    ));
    let clock = Arc::new(SystemClock);
    let facts = FactsCell::new(LoginFacts::unknown());
    let sources: Vec<Arc<dyn UsageSource>> = registry::build(&RegistryInputs {
        usage_log: Some(log),
        facts: Arc::clone(&facts),
        clock: clock.clone(),
        runner: Arc::clone(&runner),
        http: Arc::new(HttpClient::new().unwrap()),
        // No DeepSeek key: no request may leave this test.
        keys: Arc::new(|| KeySources {
            env: None,
            env_file: None,
        }),
    });
    let store = Arc::new(SqliteStore::open(&state.join("dashboard/history.db")).unwrap());
    let settings = Arc::new(JsonSettingsStore::new(
        state.join("dashboard/settings.json"),
    ));
    let (_signals_tx, signals) = tokio::sync::mpsc::channel(8);
    let handle = service::spawn(
        ServiceDeps {
            clock,
            timer: Arc::new(SystemTimer::new()),
            jitter: Arc::new(SystemJitter),
            sources,
            probe: Arc::new(CliProbe::new(runner)),
            facts,
            store,
            settings,
            notifier: Arc::new(Silent),
            signals,
            backfill: None,
        },
        &tokio::runtime::Handle::current(),
    );

    // Collect every snapshot until both CLIs' readings and the log reading are in.
    let mut seen: Vec<String> = Vec::new();
    let mut updates = handle.subscribe();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut complete = false;
    while Instant::now() < deadline && !complete {
        let json = serde_json::to_string(&handle.snapshot()).unwrap();
        complete = json.matches("\"used_bp\"").count() >= 3;
        seen.push(json);
        if !complete {
            let _ = tokio::time::timeout(Duration::from_millis(500), updates.changed()).await;
        }
    }
    assert!(complete, "no full reading arrived: {:?}", seen.last());
    let _ = handle.refresh(RefreshTarget::All).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    seen.push(serde_json::to_string(&handle.snapshot()).unwrap());
    let diagnostics = handle
        .diagnostics(ShellFacts {
            app_version: "0.0.0",
            tray_available: true,
        })
        .await;
    seen.push(serde_json::to_string(&diagnostics).unwrap());
    handle.shutdown().await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    for json in &seen {
        for canary in CANARIES {
            assert!(!json.contains(canary), "{canary} in an IPC value: {json}");
        }
    }
    let diag_text: String = capture
        .events()
        .into_iter()
        .map(diag::line)
        .collect::<Vec<_>>()
        .join("\n");
    for canary in CANARIES {
        assert!(!diag_text.contains(canary), "{canary} in diag output");
    }

    // Every file written by the app: raw bytes, and private modes.
    let mut files = Vec::new();
    walk(&state.join("dashboard"), &mut files);
    assert!(
        files.iter().any(|f| f.ends_with("history.db")),
        "the store wrote no history.db: {files:?}"
    );
    assert_eq!(mode(&state.join("dashboard")), 0o700);
    for file in &files {
        assert_eq!(mode(file), 0o600, "{}", file.display());
    }
    // The whole temp tree except the inputs the test itself planted.
    let mut all = Vec::new();
    walk(root.path(), &mut all);
    for file in all {
        if file.starts_with(&bin) || file == state.join("usage.jsonl") {
            continue;
        }
        let bytes = std::fs::read(&file).unwrap();
        for canary in CANARIES {
            assert!(!contains(&bytes, canary), "{canary} in {}", file.display());
        }
    }
}
