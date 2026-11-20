use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
pub(super) use crate::approval::domain::{grantable, refusal, settings_document};
use crate::backend::{ErrorKind, ProviderId};
use crate::testkit::capture::Buf;

mod always_tests;
mod approval_tests;
mod front_tests;
mod lines_tests;
mod status_tests;
mod turn_tests;

fn console(live: bool, style: Style, stream_answers: bool) -> (Console, Buf, Buf) {
    let (out, err) = (Buf::default(), Buf::default());
    let cfg = ConsoleConfig { out_style: style, err_style: style, width: 60, live, project: PathBuf::from("/proj"), stream_answers };
    (Console::new(Box::new(out.clone()), Box::new(err.clone()), cfg), out, err)
}

fn facts(provider: &str, model: &str) -> RouteFacts {
    RouteFacts { provider: provider.into(), model: model.into(), task: "explain".into(), reason: "task: explain (0.9) → x; first turn".into(), ..RouteFacts::default() }
}

fn quota() -> ErrorKind {
    ErrorKind::Quota { resets_at: Some("5pm".into()) }
}

/// A terminal whose width the test changes, and that remembers echo switches.
#[derive(Clone)]
struct FakeScreen {
    cols: Arc<AtomicUsize>,
    /// The echo switches made, in order.
    echo: Arc<Mutex<Vec<bool>>>,
    /// Whether the terminal echoes now (it follows the switches).
    echoing: Arc<Mutex<Option<bool>>>,
}

impl FakeScreen {
    fn new(cols: usize) -> Self {
        Self { cols: Arc::new(AtomicUsize::new(cols)), echo: Arc::default(), echoing: Arc::new(Mutex::new(Some(true))) }
    }

    fn starting_with_echo(self, echo: Option<bool>) -> Self {
        *self.echoing.lock().unwrap() = echo;
        self
    }
}

impl Screen for FakeScreen {
    fn columns(&self) -> Option<usize> {
        Some(self.cols.load(Ordering::SeqCst))
    }
    fn echo(&self) -> Option<bool> {
        *self.echoing.lock().unwrap()
    }
    fn set_echo(&self, on: bool) {
        self.echo.lock().unwrap().push(on);
        *self.echoing.lock().unwrap() = Some(on);
    }
}

fn on_screen(cols: usize) -> (Console, Buf, FakeScreen) {
    let (c, _, err) = console(true, Style::off(), true);
    let screen = FakeScreen::new(cols);
    (c.with_screen(Box::new(screen.clone())), err, screen)
}

/// The status frames drawn since the last clear: each is what follows an erase-row.
fn frames(drawn: &str) -> Vec<String> {
    drawn.split(uifmt::clear_line()).skip(1).map(|f| f.replace(uifmt::hide_cursor(), "")).collect()
}

const LONG_COMMAND: &str = "Bash ls -R | head -100 && echo --- && ls | wc -l && cat log --oneline --decorate --graph --all | head -50 && echo done and more and more text";

fn request_of(tool: &str, input: serde_json::Value) -> PermissionRequest {
    PermissionRequest { id: "q".into(), tool: tool.into(), summary: String::new(), input, blocked_path: None }
}

fn request(tool: &str, command: &str) -> PermissionRequest {
    request_of(tool, serde_json::json!({ "command": command }))
}

/// A key reader that presses `key` and records whether "always" was on offer.
fn presses(key: AnswerKey) -> (KeyReader, Arc<Mutex<Vec<bool>>>) {
    let offered = Arc::new(Mutex::new(Vec::new()));
    let seen = offered.clone();
    (Arc::new(move |always| {
        seen.lock().unwrap().push(always);
        Box::pin(std::future::ready(key))
    }), offered)
}

/// Run a future to its end on a runtime of its own.
fn wait<T>(f: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(f)
}

/// What the engine offers for `req` in this console's project, with home `/home/u` and
/// the temp folder `/tmp`.
fn offer_for(c: &Console, req: &PermissionRequest) -> Offer {
    let places = grantable::Places { project: c.lock().cfg.project.clone(), home: Some("/home/u".into()), temp: "/tmp".into() };
    always_plan::always_plan(&req.tool, &req.input, req.blocked_path.as_deref(), &places)
}

/// What the engine does with an answer: settle (save) it, then tell the console.
fn settle_like_the_engine(c: &mut Console, req: &PermissionRequest, offer: &Offer, answer: Option<Decision>) -> Option<Decision> {
    if matches!(&answer, Some(Decision::Deny { reason }) if reason == backend::NO_ONE_TO_ASK) {
        return answer;
    }
    let project = c.lock().cfg.project.clone();
    let settled = crate::turn::application::ports::settle(answer, offer, &project);
    c.permission_settled(req, &settled);
    crate::turn::application::ports::settled_decision(&settled)
}

/// The question and the person's answer, start to end, settled as the engine does.
fn ask(c: &mut Console, req: &PermissionRequest) -> Option<Decision> {
    let offer = offer_for(c, req);
    let answer = wait(c.ask_permission(req, &offer));
    settle_like_the_engine(c, req, &offer, answer)
}

fn ask_paged(c: &Console, req: &PermissionRequest, pager: Pager) -> Option<Decision> {
    let offer = offer_for(c, req);
    let answer = wait(c.ask_with_pager(req, &offer, pager));
    settle_like_the_engine(&mut c.clone(), req, &offer, answer)
}

/// A console that answers `key`, and the stderr it writes.
fn answering(key: AnswerKey) -> (Console, Buf) {
    let (c, _, err) = console(false, Style::off(), true);
    (c.with_key_reader(presses(key).0), err)
}

/// A pager that never runs: tests say whether one was available and keep what it was given.
fn no_pager() -> Pager {
    Arc::new(|_| false)
}

fn tmp_project(name: &str) -> PathBuf {
    crate::testkit::scratch_dir::scratch_dir("console", name)
}
