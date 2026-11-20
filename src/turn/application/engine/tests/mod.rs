//! The engine under test, with fake CLIs and a recording UI; the tests follow their subjects.

mod approval;
mod cancel;
mod changes;
mod failover;
mod handoff;
mod orchestration;
mod persistence;
mod reporting;
mod routing;
mod turn_log;

/// What the user asked, without the note every non-Claude prompt ends with.
fn asked(prompt: &str) -> &str {
    prompt.strip_suffix(crate::turn::domain::background::with_note("").as_str()).unwrap_or(prompt)
}

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::catalog::domain::model_table::{MODEL_MID, MODEL_STRONG};
use crate::backend::{
    self, Decision, ErrorKind, Event, McpAttachment, PermissionRequest, ProviderId,
    TurnRequest, Usage,
};
use crate::turn::domain::failover_policy::MAX_ATTEMPTS;
use crate::turn::domain::turn_status::TurnStatus;
use crate::catalog::domain::model_table as catalog;
use crate::catalog::domain::model_table::find;
use crate::changes::domain::undo_report::UndoError;
use crate::session::domain::catch_up::ResumeError;
use crate::approval::domain::mode::Mode;
use crate::orchestration::domain::orchestration_gate::Setup;
use crate::approval::domain::always_plan::{Offer, Settled};
use crate::providers::application::provider_registry::Runtime;
use crate::judgment::application::judge::{CodeWant, ProviderWant, Router};
use crate::routing::domain::decide::{Pin, Signals};
use serde_json::{json, Value};
use crate::turn::application::cancel::Cancel;
use crate::turn::application::judged::Judged;
use crate::turn::application::ports::{Answer, Ui};
use crate::turn::application::turn_report::TurnReport;
use crate::turn::domain::failover::{Failover, NoRouteInfo};
use crate::turn::domain::route_facts::RouteFacts;
use crate::turn::infrastructure::git_status::git_status_short;
use crate::backend::{idle_timeout, BackendFactory};
use super::attempt::with_steered;
use super::choose_model::{legacy_spec, LEGACY_TIERS};
use super::Engine;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::process::Command;

use crate::providers::domain::availability::{Availability, State};
use crate::backend::testing::{Attached, ScriptedCli, SessionId};
use crate::backend::Backend;
use crate::backend::classify_common;
use crate::catalog::domain::model_table::{default_chain, pick, Allowances, TaskType, Usable};
use crate::judgment::application::judge::Security;
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use ProviderId::{Claude, Codex, DeepSeek, Gemini, Grok};

const NOW: u64 = 1_760_000_000_000;
const OK: &str = "printf 'TOOL:Edit a.rs\\nFIN:the answer\\n'";
const QUOTA: &str = "printf 'FAIL:usage limit reached|1794853451\\n'";
const NOT_ENTITLED: &str = "printf 'FAIL:model not available on your plan\\n'";
const AUTH: &str = "printf 'FAIL:401 Unauthorized\\n'";
const EMPTY: &str = "printf 'FIN:\\n'";
const OTHER: &str = "printf 'FAIL:segmentation fault\\n'";
const STALL: &str = "sleep 5";
const UNUSABLE_HERE: &str = "printf 'FAIL:Not inside a trusted directory and --skip-git-repo-check was not specified.\\n'";
const FAILS_AFTER_TALKING: &str =
    "printf 'TOOL:Read a.rs\\nTEXT:let me try this approach\\nFAIL:usage limit reached|1794853451\\n'";
const TALKS_THEN_ANSWERS: &str = "printf 'TEXT:first\\nTOOL:Edit b.rs\\nTEXT:second\\nFIN:final\\n'";
const ANSWER_ALREADY_SAID: &str = "printf 'TEXT:thinking\\nTEXT:done\\nFIN:done\\n'";

/// A "CLI" that is a shell script, with a grammar of canned lines (see backend tests).
/// `clock` is the rig's clock, and `attempt_ms` how long (ms) an attempt takes before it
/// reports a failure. Like the real CLIs, claude, deepseek (through claude) and codex can
/// take an MCP server; gemini and grok cannot.
fn fake_cli(
    provider: ProviderId,
    script: &'static str,
    started: &Arc<Mutex<Vec<Started>>>,
    attached: &Attached,
    clock: &Arc<AtomicU64>,
    attempt_ms: &Arc<AtomicU64>,
) -> ScriptedCli {
    let (started, clock, attempt_ms) = (started.clone(), clock.clone(), attempt_ms.clone());
    ScriptedCli::new(provider, script)
        .session_id(SessionId::PerProvider)
        .usage(Usage { output_tokens: 7, context_tokens: Some(1000), ..Usage::default() })
        .recording_mcp(attached.clone())
        .on_start(move |req| {
            started.lock().unwrap().push(Started {
                provider,
                model: req.model.clone(),
                prompt: req.prompt.clone(),
                session: req.session_id.clone(),
            });
        })
        .on_fail(move || {
            clock.fetch_add(attempt_ms.load(Ordering::SeqCst), Ordering::SeqCst);
        })
}

#[derive(Debug, Clone)]
struct Started {
    provider: ProviderId,
    model: String,
    prompt: String,
    session: Option<String>,
}

#[derive(Default)]
struct Recorder {
    routes: Vec<String>,
    texts: Vec<String>,
    notes: Vec<String>,
    /// Everything in the order the Ui saw it: "route:", "tool:", "text:", "note:".
    seen: Vec<String>,
    /// Cancel this as soon as a tool call is shown, which is deterministic where a timer is not.
    cancel_on_tool: Option<Cancel>,
    /// Cancel as soon as a note is shown: the failover note, which falls between two attempts.
    cancel_on_note: Option<Cancel>,
}

impl Ui for Recorder {
    fn route(&mut self, line: &str) {
        self.routes.push(line.into());
        self.seen.push("route:".into());
    }
    fn event(&mut self, e: &Event) {
        match e {
            Event::Text(t) => {
                self.texts.push(t.clone());
                self.seen.push(format!("text:{t}"));
            }
            Event::Tool(t) => {
                self.seen.push(format!("tool:{t}"));
                if let Some(c) = &self.cancel_on_tool {
                    c.cancel();
                }
            }
            Event::Hint(h) => self.seen.push(format!("hint:{h}")),
            _ => {}
        }
    }
    fn note(&mut self, line: &str) {
        self.notes.push(line.into());
        self.seen.push("note:".into());
        if let Some(c) = &self.cancel_on_note {
            c.cancel();
        }
    }
}

struct Rig {
    engine: Engine,
    scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>>,
    started: Arc<Mutex<Vec<Started>>>,
    attached: Arc<Mutex<Vec<(ProviderId, McpAttachment)>>>,
    factory_calls: Arc<Mutex<Vec<ProviderId>>>,
    clock: Arc<AtomicU64>,
    /// How long (ms on `clock`) a fake attempt runs before it reports a failure.
    attempt_ms: Arc<AtomicU64>,
    ui: Recorder,
}

fn availability(provider: ProviderId, state: State) -> Availability {
    Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
}

fn rig_with(states: &[(ProviderId, State)], script: &'static str) -> Rig {
    rig_in(std::env::temp_dir(), states, script)
}

/// A rig whose engine works in `dir`. Change tracking is off unless a test turns it on.
fn rig_in(dir: PathBuf, states: &[(ProviderId, State)], script: &'static str) -> Rig {
    let avail = states.iter().map(|(p, s)| availability(*p, *s)).collect();
    let router = Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0);
    let scripts: Arc<Mutex<HashMap<ProviderId, &'static str>>> = Arc::new(Mutex::new(
        [Claude, Codex, Gemini, Grok, DeepSeek].into_iter().map(|p| (p, script)).collect(),
    ));
    let started = Arc::new(Mutex::new(Vec::new()));
    let attached = Arc::new(Mutex::new(Vec::new()));
    let factory_calls = Arc::new(Mutex::new(Vec::new()));
    let clock = Arc::new(AtomicU64::new(NOW));
    let attempt_ms = Arc::new(AtomicU64::new(0));
    let (s, st, fc, ck, at, fk, am) = (
        scripts.clone(),
        started.clone(),
        factory_calls.clone(),
        clock.clone(),
        attached.clone(),
        clock.clone(),
        attempt_ms.clone(),
    );
    let engine = Engine::new(Runtime::for_tests(avail), router, dir)
        .with_track_changes(false)
        .with_persistence(None)
        .with_backend_factory(Box::new(move |p| {
            fc.lock().unwrap().push(p);
            let script = s.lock().unwrap()[&p];
            Box::new(fake_cli(p, script, &st, &at, &fk, &am))
        }))
        .with_git_status(Box::new(|_| " M src/a.rs\n".to_string()))
        .with_clock(Box::new(move || ck.load(Ordering::SeqCst)))
        .with_idle_timeout(Duration::from_millis(300))
        .with_turn_log(None);
    Rig { engine, scripts, started, attached, factory_calls, clock, attempt_ms, ui: Recorder::default() }
}

fn states_for(ready: &[ProviderId]) -> Vec<(ProviderId, State)> {
    let all = [Claude, DeepSeek, Codex, Gemini, Grok];
    all.iter().map(|p| (*p, if ready.contains(p) { State::Ready } else { State::NotLoggedIn })).collect()
}

fn rig(ready: &[ProviderId]) -> Rig {
    rig_with(&states_for(ready), OK)
}

impl Rig {
    fn script(&self, p: ProviderId, s: &'static str) {
        self.scripts.lock().unwrap().insert(p, s);
    }

    async fn turn(&mut self, prompt: &str, w: ProviderWant) -> TurnReport {
        self.engine.run_judged(prompt, Judged::Providers(Some(w)), &mut self.ui).await
    }

    fn started(&self) -> Vec<Started> {
        self.started.lock().unwrap().clone()
    }
}

fn want(task: TaskType) -> ProviderWant {
    ProviderWant {
        task,
        task_confidence: 0.9,
        very_hard_p: 0.0,
        security_p: 0.0,
        edit_p: 0.3,
        continues_p: 0.9,
        correction_p: None,
        orchestration: crate::judgment::application::judge::Orchestration::default(),
        security_keywords: Security::None,
        signals: json!({ "task": task.as_str() }),
    }
}

/// A confident new task, which is what lets routing change provider.
fn new_task(task: TaskType) -> ProviderWant {
    ProviderWant { continues_p: 0.1, ..want(task) }
}

const LONG: &str = "please look at the whole module and change how errors are reported to callers";

/// The provider the catalog puts first for `task` when only `ready` are usable.
fn first_choice(task: TaskType, ready: &[ProviderId]) -> ProviderId {
    let usable = Usable::all_of(ready.iter().copied());
    pick(&default_chain(task, false), task, &usable, &Allowances::default(), 0).expect("usable").spec.provider
}

use crate::testkit::git_repo::git;

struct Repo(PathBuf);

impl Repo {
    fn new(name: &str, git_init: bool) -> Self {
        let dir = std::env::temp_dir().join(format!("jev-turn-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("b.txt"), "bee\n").unwrap();
        std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
        if git_init {
            git(&dir, &["init", "-q"]);
            git(&dir, &["add", "-A"]);
            git(&dir, &["commit", "-q", "-m", "init"]);
        }
        Self(dir)
    }
    fn read(&self, f: &str) -> String {
        std::fs::read_to_string(self.0.join(f)).unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Talks, then hangs: a turn that only a cancel can end quickly.
const HANGS: &str = "printf 'TOOL:Read a.rs\\nTEXT:half an answer\\n'; sleep 30";
