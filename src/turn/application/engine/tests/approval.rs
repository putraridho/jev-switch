//! Tool approvals asked of the person through the UI.

use super::*;
use crate::approval::application::claude_settings_file;
use crate::approval::domain::{always_plan, settings_document};

// ------------------------------------------------------------ tool approvals

/// An interactive "CLI": asks to run `cargo test --quiet` through Bash and reports the answer.
struct AskFake(&'static str);

const ASK_SCRIPT: &str = r#"read p; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#DENY:}";; esac"#;

impl Backend for AskFake {
    fn provider(&self) -> ProviderId {
        Claude
    }
    fn command(&self, req: &TurnRequest) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(self.0).current_dir(&req.dir);
        c
    }
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        Some(format!("{}\n", req.prompt))
    }
    fn interactive(&self) -> bool {
        true
    }
    fn parse_line(&self, line: &str) -> Vec<Event> {
        match line.trim_end().split_once(':') {
            Some(("ASK", rest)) => {
                let (tool, summary) = rest.split_once(':').unwrap_or((rest, ""));
                vec![Event::PermissionRequest(PermissionRequest {
                    id: "q1".into(),
                    tool: tool.into(),
                    summary: summary.into(),
                    input: json!({ "command": summary }),
                    blocked_path: None,
                })]
            }
            Some(("WITHDRAW", id)) => vec![Event::PermissionWithdrawn { id: id.into() }],
            Some(("FIN", t)) => vec![Event::Finished(backend::Finish { text: t.into(), session_id: None, usage: Usage::default() })],
            _ => vec![],
        }
    }
    fn permission_response(&self, _: &PermissionRequest, d: &Decision) -> Option<String> {
        Some(match d {
            Decision::Allow | Decision::AllowAlways => "ALLOW".to_string(),
            Decision::Deny { reason } => format!("DENY:{reason}"),
        })
    }
    fn classify_error(&self, m: &str) -> ErrorKind {
        classify_common(m).unwrap_or(ErrorKind::Other)
    }
}

/// A UI that answers questions from a list, and records what it was asked and told.
#[derive(Default)]
struct Asked {
    answers: std::collections::VecDeque<Option<Decision>>,
    questions: Vec<(String, String, Option<String>)>,
    settled: Vec<Settled>,
    texts: Vec<String>,
    notes: Vec<String>,
    hints: Vec<String>,
}

impl Ui for Asked {
    fn route(&mut self, _: &str) {}
    fn event(&mut self, e: &Event) {
        match e {
            Event::Text(t) => self.texts.push(t.clone()),
            Event::Hint(h) => self.hints.push(h.clone()),
            _ => {}
        }
    }
    fn note(&mut self, line: &str) {
        self.notes.push(line.into());
    }
    fn ask_permission(&mut self, req: &PermissionRequest, offer: &Offer) -> Answer {
        let rule = offer.as_ref().ok().and_then(|a| a.rules.first().cloned());
        self.questions.push((req.tool.clone(), req.summary.clone(), rule));
        let answer = self.answers.pop_front().expect("an answer was planned");
        Box::pin(std::future::ready(answer))
    }
    fn permission_settled(&mut self, req: &PermissionRequest, settled: &Settled) {
        self.settled.push(settled.clone());
        if let Settled::NotSaved(e) = settled {
            self.note(&format!("[permissions] {e}; allowed this once, nothing was saved ({})", req.tool));
        }
    }
}

/// A UI that keeps the default `ask_permission`: nobody to ask.
#[derive(Default)]
struct Mute {
    texts: Vec<String>,
    hints: Vec<String>,
}

impl Ui for Mute {
    fn route(&mut self, _: &str) {}
    fn event(&mut self, e: &Event) {
        match e {
            Event::Text(t) => self.texts.push(t.clone()),
            Event::Hint(t) => self.hints.push(t.clone()),
            _ => {}
        }
    }
    fn note(&mut self, _: &str) {}
}

fn ask_engine(name: &str) -> (Engine, PathBuf) {
    ask_engine_running(name, Box::new(|_| Box::new(AskFake(ASK_SCRIPT))))
}

fn ask_engine_running(name: &str, backend: BackendFactory) -> (Engine, PathBuf) {
    let dir = std::env::temp_dir().join(format!("jev-turn-ask-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let router = Router::new(TypeSafe::with_url("k", "http://127.0.0.1:1"), 1.0);
    let engine = Engine::new(Runtime::for_tests(vec![availability(Claude, State::Ready)]), router, dir.clone(), crate::turn::infrastructure::engine_adapters::real())
        .with_track_changes(false)
        .with_persistence(None)
        .with_backend_factory(backend)
        .with_idle_timeout(Duration::from_secs(5))
        .with_turn_log(None);
    (engine, dir)
}

async fn ask_turn(engine: &mut Engine, ui: &mut dyn Ui) -> TurnReport {
    engine.run_judged("run the tests", Judged::Providers(Some(want(TaskType::Feature))), ui).await
}

#[tokio::test]
async fn an_allowed_tool_call_runs_and_the_person_was_asked_once_with_the_rule_on_offer() {
    let (mut engine, dir) = ask_engine("allow");
    let mut ui = Asked { answers: [Some(Decision::Allow)].into(), ..Asked::default() };
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
    assert_eq!(ui.questions, vec![("Bash".to_string(), "cargo test --quiet".to_string(), Some("Bash(cargo test:*)".to_string()))]);
    assert!(!settings_document::local_settings_path(&dir).exists(), "a plain yes saves nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn auto_mode_runs_a_safe_call_without_asking_and_saves_nothing() {
    let (engine, dir) = ask_engine("auto-safe");
    let mut engine = engine.with_mode(Mode::Auto);
    let mut ui = Asked::default();
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
    assert!(ui.questions.is_empty(), "nobody was asked: {:?}", ui.questions);
    assert!(ui.hints.iter().any(|h| h.contains("auto-allowed") && h.contains("cargo test --quiet")), "{:?}", ui.hints);
    assert!(!settings_document::local_settings_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn auto_mode_allows_a_call_that_cannot_be_remembered() {
    let script = r#"read p; echo "ASK:Bash:git push"; read ans; case "$ans" in ALLOW) echo "FIN:ran";; *) echo "FIN:refused";; esac"#;
    let (engine, dir) = ask_engine_running("auto-push", Box::new(move |_| Box::new(AskFake(script))));
    let mut engine = engine.with_mode(Mode::Auto);
    let mut ui = Asked::default();
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
    assert!(ui.questions.is_empty());
    assert!(ui.hints.iter().any(|h| h.contains("auto-allowed") && h.contains("git push")));
    assert!(!settings_document::local_settings_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_other_modes_still_ask_about_the_same_call() {
    for mode in [Mode::Ask, Mode::Edit, Mode::Plan] {
        let (engine, dir) = ask_engine(&format!("ask-{mode}"));
        let mut engine = engine.with_mode(mode);
        let mut ui = Asked { answers: [Some(Decision::Allow)].into(), ..Asked::default() };
        ask_turn(&mut engine, &mut ui).await;
        assert_eq!(ui.questions.len(), 1, "{mode}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tokio::test]
async fn always_runs_the_call_and_saves_the_rule_in_the_project() {
    let (mut engine, dir) = ask_engine("always");
    let mut ui = Asked { answers: [Some(Decision::AllowAlways)].into(), ..Asked::default() };
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!(report.text, "ran");
    let saved = claude_settings_file::load(&settings_document::local_settings_path(&dir)).unwrap();
    assert_eq!(saved.list(settings_document::List::Allow), ["Bash(cargo test:*)"]);
    assert!(ui.notes.is_empty(), "{:?}", ui.notes);
    let always = always_plan::Always { rules: vec!["Bash(cargo test:*)".into()], folder: None };
    assert_eq!(ui.settled, [Settled::Remembered(always)], "the UI hears what was saved, after the save");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn always_that_cannot_be_saved_still_allows_once_and_says_so() {
    let (mut engine, dir) = ask_engine("always-blocked");
    // `.claude` is a file, so the settings file cannot be created.
    std::fs::write(dir.join(".claude"), "x").unwrap();
    let mut ui = Asked { answers: [Some(Decision::AllowAlways)].into(), ..Asked::default() };
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!(report.text, "ran");
    assert!(ui.notes.iter().any(|n| n.contains("[permissions]") && n.contains("nothing was saved")), "{:?}", ui.notes);
    assert!(matches!(ui.settled.as_slice(), [Settled::NotSaved(_)]), "never told 'always' when nothing was saved: {:?}", ui.settled);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_denied_tool_call_tells_the_model_why_and_the_turn_goes_on() {
    let (mut engine, dir) = ask_engine("deny");
    let denial = Decision::Deny { reason: "the user denied this tool call".into() };
    let mut ui = Asked { answers: [Some(denial)].into(), ..Asked::default() };
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "refused the user denied this tool call"));
    assert!(!settings_document::local_settings_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ctrl_c_at_the_question_cancels_the_turn_and_kills_the_cli() {
    let (mut engine, dir) = ask_engine("interrupt");
    let mut ui = Asked { answers: [None].into(), ..Asked::default() };
    let cancel = Cancel::new();
    let report = engine
        .run_judged_cancellable("run the tests", Judged::Providers(Some(want(TaskType::Feature))), &mut ui, &cancel)
        .await;
    assert_eq!(report.status, TurnStatus::Cancelled);
    assert!(cancel.is_cancelled());
    assert!(report.text.is_empty());
    assert_eq!(ui.questions.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_ui_that_cannot_ask_denies_at_once_with_a_hint_and_never_hangs() {
    let (mut engine, dir) = ask_engine("mute");
    let mut ui = Mute::default();
    let started = Instant::now();
    let report = ask_turn(&mut engine, &mut ui).await;
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(report.status, TurnStatus::Answered);
    assert!(report.text.starts_with("refused no one to ask"), "{}", report.text);
    assert!(ui.hints.iter().any(|t| t.contains("denied Bash") && t.contains("CODE_ALLOWED_TOOLS")), "{:?}", ui.hints);
    assert!(ui.texts.iter().all(|t| !t.contains("denied Bash")), "a hint is never answer text: {:?}", ui.texts);
    assert!(!settings_document::local_settings_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn code_allowed_tools_answers_a_cli_that_asks_the_host_even_with_nobody_to_ask() {
    let (engine, dir) = ask_engine("allowed-setting");
    let mut engine = engine.with_allowed_tools("Bash(cargo test:*)");
    let mut ui = Mute::default();
    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
    assert!(ui.hints.iter().any(|t| t.contains("allowed (CODE_ALLOWED_TOOLS)") && t.contains("cargo test")), "{:?}", ui.hints);
    // Plan mode changes nothing, so the rules do not apply there: still denied.
    let mut plan = engine.with_allowed_tools("Bash(cargo test:*)");
    plan.set_mode(Mode::Plan);
    let report = ask_turn(&mut plan, &mut Mute::default()).await;
    assert!(report.text.starts_with("refused no one to ask"), "{}", report.text);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn backends_without_a_host_protocol_are_never_asked() {
    let mut r = rig(&[Claude]);
    let mut ui = Asked::default();
    let report = r.engine.run_judged("add a retry", Judged::Providers(Some(want(TaskType::Feature))), &mut ui).await;
    assert_eq!(report.status, TurnStatus::Answered);
    assert!(ui.questions.is_empty());
}

/// Records that the wait for the person's answer was dropped.
struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A UI whose first question waits for a key that never comes (the person is thinking),
/// and which answers yes to any later one. It records the wait being dropped and the
/// withdrawal it was told about.
#[derive(Default)]
struct Waiting {
    asked: usize,
    dropped: Arc<std::sync::atomic::AtomicBool>,
    withdrawn: Vec<String>,
}

impl Ui for Waiting {
    fn route(&mut self, _: &str) {}
    fn event(&mut self, _: &Event) {}
    fn note(&mut self, _: &str) {}
    fn ask_permission(&mut self, _: &PermissionRequest, _: &Offer) -> Answer {
        self.asked += 1;
        if self.asked > 1 {
            return Box::pin(std::future::ready(Some(Decision::Allow)));
        }
        let flag = DropFlag(self.dropped.clone());
        Box::pin(async move {
            let _waiting = flag;
            std::future::pending::<Option<Decision>>().await
        })
    }
    fn permission_withdrawn(&mut self, req: &PermissionRequest) {
        self.withdrawn.push(req.id.clone());
    }
}

/// The CLI asks, the person has not answered yet, and the CLI withdraws the question.
const ASK_THEN_WITHDRAW: &str = r#"read p; echo "ASK:Bash:cargo test --quiet"; sleep 0.3; echo "WITHDRAW:q1"; sleep 0.1; echo "FIN:carried on""#;

#[tokio::test]
async fn a_question_the_cli_withdraws_while_the_person_is_still_deciding_is_dropped_and_the_next_turn_works() {
    let turns = Arc::new(AtomicU64::new(0));
    let counted = turns.clone();
    let (mut engine, dir) = ask_engine_running(
        "withdrawn",
        Box::new(move |_| Box::new(AskFake(if counted.fetch_add(1, Ordering::SeqCst) == 0 { ASK_THEN_WITHDRAW } else { ASK_SCRIPT }))),
    );
    let mut ui = Waiting::default();
    let started = Instant::now();
    let report = ask_turn(&mut engine, &mut ui).await;
    assert!(started.elapsed() < Duration::from_secs(4), "the turn must not wait for a key");
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "carried on"));
    assert_eq!(ui.asked, 1);
    assert!(ui.dropped.load(Ordering::SeqCst), "the wait for a key was abandoned");
    assert_eq!(ui.withdrawn, vec!["q1".to_string()], "the UI was told which question went away");
    assert!(!settings_document::local_settings_path(&dir).exists());

    let report = ask_turn(&mut engine, &mut ui).await;
    assert_eq!((report.status, report.text.as_str()), (TurnStatus::Answered, "ran"));
    assert_eq!(ui.asked, 2);
    assert_eq!(ui.withdrawn.len(), 1, "an answered question is never reported as withdrawn");
    let _ = std::fs::remove_dir_all(&dir);
}
