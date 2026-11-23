//! The REPL through fake model CLIs: the rig every test file here uses.

use super::*;
use super::turn_race::{Interrupt, TurnEnd};
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};
use jev_turn::application::engine::Engine;
use crate::render::Style;
use crate::console::terminal_ui::Console;
use jev_providers::domain::availability::{Availability, State};
use jev_backend::{ProviderId, Usage};
use jev_backend::testing::ScriptedCli;
use jev_testkit::capture::Buf;
use crate::console::terminal_ui::ConsoleConfig;
use jev_providers::application::provider_registry::Runtime;
use jev_judgment::application::judge::Router;
use jev_turn::application::judged::Judged;
use jev_judgment::infrastructure::typesafe_http::TypeSafe;
use std::sync::{Arc, Mutex};

mod diff_tests;
mod extension_tests;
mod project_tests;
mod session_tests;
mod turn_tests;

fn fake_cli(provider: ProviderId, script: &str) -> ScriptedCli {
    ScriptedCli::new(provider, script)
        .usage(Usage { input_tokens: 2000, cached_input_tokens: 1000, output_tokens: 50, context_tokens: Some(2000), ..Usage::default() })
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

pub struct Rig {
    pub repl: Repl,
    pub out: Buf,
    pub err: Buf,
    pub dir: std::path::PathBuf,
}

pub fn rig(name: &str, script: &str) -> Rig {
    let dir = jev_testkit::scratch_dir::scratch_dir("repl", name);
    jev_testkit::git_repo::init_with_commit(&dir, &[("a.txt", "one\n")]);
    let avail = vec![Availability { provider: ProviderId::Claude, state: State::Ready, plan: None, models: None, detail: String::new(), verified: true }];
    let script = script.to_string();
    let engine = Engine::new(Runtime::for_tests(avail), Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0), dir.clone(), jev_turn::infrastructure::engine_adapters::real())
        .with_working_tree(std::sync::Arc::new(jev_changes::infrastructure::git_tree::GitWorkingTree))
        .with_turn_log(None)
        .with_backend_factory(Box::new(move |p| Box::new(fake_cli(p, &script))));
    let (out, err) = (Buf::default(), Buf::default());
    let cfg = ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: dir.clone(), stream_answers: true };
    let console = Console::new(Box::new(out.clone()), Box::new(err.clone()), cfg);
    Rig { repl: Repl::new(engine, console, None, false), out, err, dir }
}

pub async fn run(r: &mut Rig, line: &str) -> Flow {
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.handle(crate::command::parse(line), &mut io).await
}

pub const ANSWER: &str = "printf 'TOOL:Edit a.txt\\nTEXT:working\\nFIN:the answer\\n'; echo two >> a.txt";

async fn judged_turn(r: &mut Rig, prompt: &str) -> TurnEnd {
    let want = jev_judgment::domain::want::ProviderWant {
        task: jev_catalog::domain::model_table::TaskType::Feature,
        task_confidence: 0.9,
        very_hard_p: 0.0,
        security_p: 0.0,
        edit_p: 0.5,
        continues_p: 0.5,
        correction_p: None,
        orchestration: jev_judgment::domain::orchestration::Orchestration::default(),
        security_keywords: jev_judgment::domain::security::Security::None,
        signals: serde_json::json!({}).into(),
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
    r.repl.handle(crate::command::parse(line), &mut io).await;
}

