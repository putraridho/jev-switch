//! The delegator under test, with fake worktrees, fake CLIs and a fake test runner; the pieces
//! that talk to real git or real processes are tested beside their own files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use jev_backend::Mode;
use jev_backend::testing::{ScriptedCli, SessionId};
use jev_backend::{ErrorKind, ProviderId, Usage};
use jev_backend::ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};
use jev_catalog::domain::model_table::TaskType;
use jev_judgment::domain::security::Security;
use jev_judgment::domain::want::ProviderWant;
use crate::domain::delegation::*;
use jev_providers::domain::availability::{Availability, State};
use jev_backend::{TurnError, TurnOutcome};
use jev_catalog::domain::model_table::models;
use crate::application::delegator::background::task_number;
use crate::domain::recovery::{Fate, Found};
use crate::application::ports::AttemptLog;
use crate::infrastructure::adapters;
use crate::infrastructure::delegation_log::{handoff_key, recent_delegations};
use crate::infrastructure::worker_file::Store;
use crate::application::ports::{Screen, WorkerPolicy};
use crate::infrastructure::git_worktrees::GitWorktrees;
use crate::application::ports::CheckRun;
use crate::infrastructure::worker_file::{has_leftovers, open_workers, plain_id};
use jev_testkit::unified_patch::{big_patch, file_patch};
use jev_shared::application::host::Host as _;

/// The machine's clock, for records that must look real to recovery.
fn system_now_ms() -> u64 {
    jev_shared::infrastructure::system_host::SystemHost.now_ms()
}
use anyhow::bail;
use std::time::Instant;
use super::*;

mod background;
mod cancellation;
mod delegate;
mod persistence;
mod review;
mod ids_and_logs;

const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];
const NOW: u64 = 1_760_000_000_000;

/// A "CLI" that is a shell script with canned lines: `FIN:text`, `FAIL:message`.
/// `started` gets (provider, model, dir, prompt) and `modes` the permission mode of every
/// turn started, in order.
fn fake_cli(provider: ProviderId, script: &'static str, started: &Started, modes: &Modes) -> ScriptedCli {
    let (started, modes) = (started.clone(), modes.clone());
    ScriptedCli::new(provider, script).session_id(SessionId::Absent).on_start(move |req| {
        started.lock().unwrap().push((provider, req.model.clone(), req.dir.clone(), req.prompt.clone()));
        modes.lock().unwrap().push(req.mode);
    })
}

type Modes = Arc<Mutex<Vec<Mode>>>;

const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:did the piece\\n'";
const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";

/// Directories made and removed, and what `apply` was given; no git involved.
#[derive(Default)]
struct FakeWorktrees {
    base: PathBuf,
    created: Mutex<Vec<PathBuf>>,
    removed: Mutex<Vec<PathBuf>>,
    applied: Mutex<Vec<String>>,
    unapplied: Mutex<Vec<String>>,
    pruned: Mutex<Vec<PathBuf>>,
    apply_fails: bool,
    unapply_fails: bool,
    create_fails: bool,
    /// Every `create` and `apply` takes this long, like a big checkout would.
    delay: Duration,
    /// What `diff` returns for every worker; by default a one-file patch named after it.
    patch: Mutex<Option<String>>,
}

fn temp_dir(tag: &str) -> PathBuf {
    jev_testkit::scratch_dir::unique_scratch_dir("delegate", tag)
}

impl FakeWorktrees {
    fn new() -> Arc<Self> {
        let base = temp_dir("fake");
        // The "main tree", where a read-only worker runs in place.
        std::fs::create_dir_all(base.join("repo")).unwrap();
        Arc::new(Self { base, ..Self::default() })
    }

    fn slow(delay: Duration) -> Arc<Self> {
        Arc::new(Self { base: temp_dir("slow"), delay, ..Self::default() })
    }

    fn with_patch(self: Arc<Self>, patch: String) -> Arc<Self> {
        *self.patch.lock().unwrap() = Some(patch);
        self
    }
}

impl Worktrees for Arc<FakeWorktrees> {
    fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }
    fn root(&self) -> PathBuf {
        self.base.clone()
    }
    fn create(&self, id: &str) -> Result<PathBuf> {
        std::thread::sleep(self.delay);
        if self.create_fails {
            bail!("disk full");
        }
        let p = self.base.join(id);
        std::fs::create_dir_all(&p)?;
        self.created.lock().unwrap().push(p.clone());
        Ok(p)
    }
    fn diff(&self, path: &Path) -> Result<String> {
        if let Some(p) = self.patch.lock().unwrap().clone() {
            return Ok(p);
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(file_patch(&format!("{name}.rs"), "a", &["b"]))
    }
    fn apply(&self, patch: &str) -> Result<()> {
        std::thread::sleep(self.delay);
        if self.apply_fails {
            bail!("conflict");
        }
        self.applied.lock().unwrap().push(patch.to_string());
        Ok(())
    }
    fn unapply(&self, patch: &str) -> Result<()> {
        if self.unapply_fails {
            bail!("those files changed since");
        }
        self.unapplied.lock().unwrap().push(patch.to_string());
        Ok(())
    }
    fn remove(&self, path: &Path) -> Result<()> {
        self.removed.lock().unwrap().push(path.to_path_buf());
        let _ = std::fs::remove_dir_all(path);
        Ok(())
    }
    fn prune(&self, repo: &Path) -> Result<()> {
        self.pruned.lock().unwrap().push(repo.to_path_buf());
        Ok(())
    }
}

/// A project whose test command is `command` (none: no tests known), and whose every run
/// ends as `outcome`. Records where it ran.
struct FakeChecks {
    command: Option<&'static str>,
    outcome: CheckRun,
    ran: Arc<Mutex<Vec<PathBuf>>>,
}

impl Checks for FakeChecks {
    fn test_command(&self, _dir: &Path) -> Option<String> {
        self.command.map(String::from)
    }
    fn run(&self, dir: &Path, _command: &str, _stop: &dyn Fn() -> bool) -> CheckRun {
        self.ran.lock().unwrap().push(dir.to_path_buf());
        self.outcome.clone()
    }
}

/// `rig`'s delegator with a project whose `cargo test` ends as `outcome`; where it ran.
fn tested(mut r: Rig, outcome: CheckRun) -> (Rig, Arc<Mutex<Vec<PathBuf>>>) {
    let ran = Arc::new(Mutex::new(Vec::new()));
    r.d = r.d.with_checks(Box::new(FakeChecks { command: Some("cargo test"), outcome, ran: ran.clone() }));
    (r, ran)
}

fn want(task: TaskType) -> ProviderWant {
    ProviderWant {
        task,
        task_confidence: 0.9,
        very_hard_p: 0.0,
        security_p: 0.0,
        edit_p: 0.9,
        continues_p: 0.0,
        correction_p: None,
        orchestration: jev_judgment::domain::orchestration::Orchestration::default(),
        security_keywords: Security::None,
        signals: json!({ "task": task.as_str() }).into(),
    }
}

fn availability(provider: ProviderId, ready: bool) -> Availability {
    let state = if ready { State::Ready } else { State::NotLoggedIn };
    Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
}

type Started = Arc<Mutex<Vec<(ProviderId, String, PathBuf, String)>>>;
type Scripts = Arc<Mutex<HashMap<ProviderId, &'static str>>>;

struct Rig {
    d: Delegator,
    wt: Arc<FakeWorktrees>,
    started: Started,
    scripts: Scripts,
    modes: Modes,
}

/// A delegator on `wt` whose fake CLIs all succeed unless `scripts` says otherwise.
fn delegator_on(wt: &Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType, started: &Started, scripts: &Scripts, modes: &Modes) -> Delegator {
    let avail = PROVIDERS.iter().map(|p| availability(*p, ready.contains(p))).collect();
    let (st, sc, md) = (started.clone(), scripts.clone(), modes.clone());
    let judge: Judge = Box::new(move |_| Box::pin(async move { Some(want(task)) }));
    Delegator::new(Runtime::for_tests(avail), judge, Box::new(wt.clone()), real_adapters())
        .with_backend_factory(Box::new(move |p| {
            let script = sc.lock().unwrap()[&p];
            Box::new(fake_cli(p, script, &st, &md))
        }))
        .with_clock(Box::new(|| NOW))
        .with_idle_timeout(Duration::from_millis(500))
        .with_planner("claude:claude-opus-5-5".into())
        // No test command known: work goes to review unless a test says otherwise.
        .with_checks(Box::new(FakeChecks { command: None, outcome: CheckRun::Passed, ran: Arc::default() }))
}

fn rig_on(wt: Arc<FakeWorktrees>, ready: &[ProviderId], task: TaskType) -> Rig {
    let started: Started = Arc::new(Mutex::new(Vec::new()));
    let scripts: Scripts = Arc::new(Mutex::new(PROVIDERS.iter().map(|p| (*p, OK)).collect()));
    let modes = Modes::default();
    let d = delegator_on(&wt, ready, task, &started, &scripts, &modes);
    Rig { d, wt, started, scripts, modes }
}

fn rig(ready: &[ProviderId], task: TaskType) -> Rig {
    rig_on(FakeWorktrees::new(), ready, task)
}

fn brief(text: &str) -> DelegateRequest {
    DelegateRequest { brief: text.into(), ..DelegateRequest::default() }
}

/// A worker policy with nothing configured: every provider may have a brief, prompts leave
/// unchanged (the briefs in these tests hold no secret), and the request is the plain one.
/// The real policy (privacy, secret scan, audit) is the turn's and is tested there.
struct OpenPolicy;

impl WorkerPolicy for OpenPolicy {
    fn screen(&self, _repo: &Path, _outgoing: &str, brief: &str) -> Screen {
        Screen { permitted: jev_shared::ALL_PROVIDERS.to_vec(), refusal: String::new(), judge_prompt: Some(brief.to_string()) }
    }

    fn outgoing(&self, prompt: &str, _provider: ProviderId, _model: &str, _now_ms: u64) -> String {
        prompt.to_string()
    }

    fn request(&self, dir: &Path, model: &str, prompt: String, read_only: bool) -> jev_backend::TurnRequest {
        let mode = if read_only { Mode::Plan } else { Mode::default() };
        jev_backend::TurnRequest { dir: dir.to_path_buf(), model: model.to_string(), prompt, images: vec![], session_id: None, mode, trust_project_settings: false }
    }
}

/// The real adapters with a policy that lets every brief through.
fn real_adapters() -> crate::application::ports::Adapters {
    adapters::real(Arc::new(OpenPolicy))
}
