//! The REPL through fake model CLIs: the rig every test file here uses.

use super::*;
use super::turn_race::{Interrupt, TurnEnd};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use crate::turn::application::engine::Engine;
use crate::tui::render::Style;
use crate::tui::console::terminal_ui::Console;
use crate::providers::domain::availability::{Availability, State};
use crate::backend::{ProviderId, Usage};
use crate::backend::testing::ScriptedCli;
use crate::testkit::capture::Buf;
use crate::tui::console::terminal_ui::ConsoleConfig;
use crate::providers::application::provider_registry::Runtime;
use crate::judgment::application::judge::Router;
use crate::turn::application::judged::Judged;
use crate::judgment::infrastructure::typesafe_http::TypeSafe;
use std::sync::{Arc, Mutex};

mod diff_tests;
mod extension_tests;
mod project_tests;
mod session_tests;
mod turn_tests;

fn fake_cli(provider: ProviderId, script: &str) -> ScriptedCli {
    ScriptedCli::new(provider, script)
        .usage(Usage { input_tokens: 2000, cached_input_tokens: 1000, output_tokens: 50, context_tokens: Some(2000) })
        .with_prompt_env()
}

/// An interrupt that never fires, or fires after a delay, once or twice.
struct Presses {
    pending: Vec<Duration>,
}

impl Interrupt for Presses {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let delay = if self.pending.is_empty() { None } else { Some(self.pending.remove(0)) };
        Box::pin(async move {
            match delay {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending::<()>().await,
            }
        })
    }
    fn drain(&mut self) {}
}

struct Answer(bool, Arc<Mutex<Vec<String>>>);

impl Confirm for Answer {
    fn confirm(&mut self, question: &str) -> bool {
        self.1.lock().unwrap().push(question.to_string());
        self.0
    }
}

pub(crate) struct Rig {
    pub(crate) repl: Repl,
    pub(crate) out: Buf,
    pub(crate) err: Buf,
    pub(crate) dir: std::path::PathBuf,
}

pub(crate) fn rig(name: &str, script: &str) -> Rig {
    let dir = crate::testkit::scratch_dir::scratch_dir("repl", name);
    crate::testkit::git_repo::init_with_commit(&dir, &[("a.txt", "one\n")]);
    let avail = vec![Availability { provider: ProviderId::Claude, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true }];
    let script = script.to_string();
    let engine = Engine::new(Runtime::for_tests(avail), Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), dir.clone())
        .with_turn_log(None)
        .with_backend_factory(Box::new(move |p| Box::new(fake_cli(p, &script))));
    let (out, err) = (Buf::default(), Buf::default());
    let cfg = ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: dir.clone(), stream_answers: true };
    let console = Console::new(Box::new(out.clone()), Box::new(err.clone()), cfg);
    Rig { repl: Repl::new(engine, console, None, false), out, err, dir }
}

pub(crate) async fn run(r: &mut Rig, line: &str) -> Flow {
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.handle(crate::tui::command::parse(line), &mut io).await
}

pub(crate) const ANSWER: &str = "printf 'TOOL:Edit a.txt\\nTEXT:working\\nFIN:the answer\\n'; echo two >> a.txt";

async fn judged_turn(r: &mut Rig, prompt: &str) -> TurnEnd {
    let want = crate::judgment::application::judge::ProviderWant {
        task: crate::catalog::domain::model_table::TaskType::Feature,
        task_confidence: 0.9,
        very_hard_p: 0.0,
        security_p: 0.0,
        edit_p: 0.5,
        continues_p: 0.5,
        correction_p: None,
        orchestration: crate::judgment::application::judge::Orchestration::default(),
        security_keywords: crate::judgment::application::judge::Security::None,
        signals: serde_json::json!({}),
    };
    let started = Instant::now();
    let mut ui = r.repl.console.clone();
    r.repl.console.begin_turn();
    let report = r.repl.engine.run_judged(prompt, Judged::Providers(Some(want)), &mut ui).await;
    r.repl.console.end_turn();
    TurnEnd::Done(Box::new(report), started.elapsed())
}

/// A prompt through the REPL, routed without Jev (the test router has no server).
async fn send(r: &mut Rig, line: &str) {
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.handle(crate::tui::command::parse(line), &mut io).await;
}

