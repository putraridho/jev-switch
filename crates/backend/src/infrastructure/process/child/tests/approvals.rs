use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
#[cfg(unix)]
use super::process_group::{all_dead, read_pid};
use tokio::process::Command;

/// An interactive "CLI": a shell script that asks questions and reads the answers from
/// stdin. Output grammar: `ASK:<tool>:<summary>` (id `q1`), `ASKID:<id>:<tool>:<summary>`,
/// `CANCEL:<id>`, `TEXT:<t>`, `TOOL:<t>`, `FIN:<text>`. The answers it receives are
/// `<id>:ALLOW` (also for always) and `<id>:DENY:<reason>`.
pub(super) struct Asking(pub(super) String);

impl Backend for Asking {
    fn provider(&self) -> ProviderId {
        ProviderId::Claude
    }
    fn command(&self, req: &TurnRequest) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&self.0).current_dir(&req.dir);
        c
    }
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        Some(format!("{}\n", req.prompt))
    }
    fn interactive(&self) -> bool {
        true
    }
    fn parse_line(&self, line: &str) -> Vec<Event> {
        let line = line.trim_end();
        let ask = |id: &str, rest: &str| {
            let (tool, summary) = rest.split_once(':').unwrap_or((rest, ""));
            vec![Event::PermissionRequest(PermissionRequest {
                id: id.into(),
                tool: tool.into(),
                summary: summary.into(),
                input: jev_approval::infrastructure::tool_input_wire::tool_input_from_json(&serde_json::json!({ "command": summary })),
                blocked_path: None,
            })]
        };
        match line.split_once(':') {
            Some(("ASK", rest)) => ask("q1", rest),
            Some(("ASKID", rest)) => {
                let (id, rest) = rest.split_once(':').unwrap_or((rest, ""));
                ask(id, rest)
            }
            Some(("CANCEL", id)) => vec![Event::PermissionWithdrawn { id: id.into() }],
            Some(("TEXT", t)) => vec![Event::Text(t.into())],
            Some(("TOOL", t)) => vec![Event::Tool(t.into())],
            Some(("FIN", text)) => vec![Event::Finished(Finish { text: text.into(), session_id: None, usage: Usage::default() })],
            // How Gemini and Antigravity end an empty answer: their own failure, not a result.
            Some(("FAIL", message)) => vec![Event::Failed { message: message.into() }],
            _ => vec![],
        }
    }
    fn permission_response(&self, req: &PermissionRequest, decision: &Decision) -> Option<String> {
        Some(match decision {
            Decision::Allow | Decision::AllowAlways => format!("{}:ALLOW", req.id),
            Decision::Deny { reason } => format!("{}:DENY:{reason}", req.id),
        })
    }
    fn classify_error(&self, message: &str) -> ErrorKind {
        if message.contains("empty response") {
            ErrorKind::Empty
        } else {
            ErrorKind::Other
        }
    }
}

pub(super) const ASK_AND_REPORT: &str =
    r#"read prompt; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in q1:ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#q1:DENY:}";; esac"#;

/// An approver that answers after `wait`, and remembers what it was asked, what it was
/// told was withdrawn, and (in `log`) what happened in which order.
pub(super) struct Scripted {
    wait: Duration,
    decision: Decision,
    asked: Mutex<Vec<(String, String)>>,
    withdrawn: Mutex<Vec<String>>,
    pub(super) log: Mutex<Vec<String>>,
}

impl Scripted {
    pub(super) fn new(wait_ms: u64, decision: Decision) -> Self {
        Self {
            wait: Duration::from_millis(wait_ms),
            decision,
            asked: Mutex::new(Vec::new()),
            withdrawn: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn note(&self, what: String) {
        self.log.lock().unwrap().push(what);
    }

    pub(super) fn withdrawn_ids(&self) -> Vec<String> {
        self.withdrawn.lock().unwrap().clone()
    }
}

impl Approver for Scripted {
    fn decide<'a>(&'a self, req: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async move {
            self.asked.lock().unwrap().push((req.tool.clone(), req.summary.clone()));
            self.note(format!("asked {}", req.id));
            tokio::time::sleep(self.wait).await;
            self.note(format!("decided {}", req.id));
            self.decision.clone()
        })
    }

    fn withdrawn(&self, req: &PermissionRequest) {
        self.withdrawn.lock().unwrap().push(req.id.clone());
        self.note(format!("withdrawn {}", req.id));
    }
}

pub(super) async fn run_asking(script: &str, approver: &dyn Approver, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
    let mut seen = Vec::new();
    let mut on_event = |e: &Event| seen.push(e.clone());
    let r = run_turn_with(&Asking(script.into()), &req(), Duration::from_millis(idle_ms), &mut on_event, approver).await;
    (r, seen)
}

#[tokio::test]
async fn an_allowed_request_lets_the_cli_go_on_and_is_not_shown_as_an_event() {
    let approver = Scripted::new(0, Decision::Allow);
    let (r, seen) = run_asking(ASK_AND_REPORT, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "ran");
    assert_eq!(*approver.asked.lock().unwrap(), vec![("Bash".to_string(), "cargo test --quiet".to_string())]);
    assert!(seen.is_empty(), "{seen:?}");
}

#[tokio::test]
async fn always_reaches_the_cli_as_an_allow() {
    let (r, _) = run_asking(ASK_AND_REPORT, &Scripted::new(0, Decision::AllowAlways), 5000).await;
    assert_eq!(r.unwrap().text, "ran");
}

#[tokio::test]
async fn a_denial_carries_its_reason_to_the_cli() {
    let approver = Scripted::new(0, Decision::Deny { reason: "not today".into() });
    let (r, _) = run_asking(ASK_AND_REPORT, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "refused not today");
}

#[tokio::test]
async fn the_idle_watchdog_does_not_run_while_the_person_decides() {
    // The question takes four times the idle timeout to answer; the turn must not be cut off.
    let approver = Scripted::new(800, Decision::Allow);
    let (r, _) = run_asking(ASK_AND_REPORT, &approver, 200).await;
    assert_eq!(r.unwrap().text, "ran");
    // And it still guards the CLI's own silence afterwards.
    let (r, _) = run_asking(r#"read p; echo "ASK:Bash:x"; read a; exec sleep 30"#, &Scripted::new(0, Decision::Allow), 300).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
}

#[tokio::test]
async fn without_an_approver_every_request_is_denied_and_nothing_hangs() {
    let mut on_event = |_: &Event| {};
    let started = std::time::Instant::now();
    let r = run_turn(&Asking(ASK_AND_REPORT.into()), &req(), Duration::from_secs(5), &mut on_event).await;
    assert_eq!(r.unwrap().text, format!("refused {NO_ONE_TO_ASK}"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn an_empty_answer_after_a_denial_nobody_could_answer_ends_the_turn_instead_of_failing_over() {
    let mut on_event = |_: &Event| {};
    let script = r#"read prompt; echo "ASK:Bash:x"; read ans; echo "FIN:""#;
    let err = run_turn(&Asking(script.into()), &req(), Duration::from_secs(5), &mut on_event).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Other, "{}", err.message);
    assert!(err.message.contains("Bash was denied") && err.message.contains("CODE_ALLOWED_TOOLS"), "{}", err.message);
    // The person said no: an empty answer then is still the model's, and the next one may do better.
    let (r, _) = run_asking(script, &Scripted::new(0, Decision::Deny { reason: "no".into() }), 5000).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Empty);
    // The same when the CLI reports the empty answer as its own failure (Antigravity does).
    let failing = r#"read prompt; echo "ASK:Bash:x"; read ans; echo "FAIL:empty response""#;
    let err = run_turn(&Asking(failing.into()), &req(), Duration::from_secs(5), &mut on_event).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Other, "{}", err.message);
    assert!(err.message.contains("Bash was denied"), "{}", err.message);
}

#[tokio::test]
async fn stdin_stays_open_for_answers_and_closes_once_the_turn_has_finished() {
    let dir = scratch_dir("interactive-close");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("exited");
    // After its answer the "CLI" reads until stdin closes, then leaves a marker.
    let script = format!(
        r#"read prompt; echo "ASK:Bash:x"; read ans; echo "FIN:ok"; cat > /dev/null; touch {}"#,
        marker.display()
    );
    let started = std::time::Instant::now();
    let (r, _) = run_asking(&script, &Scripted::new(0, Decision::Allow), 5000).await;
    assert_eq!(r.unwrap().text, "ok");
    assert!(started.elapsed() < Duration::from_millis(1500), "stdin was not closed after the result: {:?}", started.elapsed());
    assert!(marker.exists(), "the CLI should have seen end of input and exited by itself");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn a_turn_dropped_while_a_question_waits_kills_the_cli() {
    let dir = scratch_dir("interactive-drop");
    std::fs::create_dir_all(&dir).unwrap();
    let pid_file = dir.join("cli.pid");
    let script = format!(r#"echo $$ > {}; read prompt; echo "ASK:Bash:x"; read ans; echo FIN:late"#, pid_file.display());
    let approver = Scripted::new(60_000, Decision::Allow);
    let request = req();
    let mut on_event = |_: &Event| {};
    let backend = Asking(script);
    let turn = run_turn_with(&backend, &request, Duration::from_millis(300), &mut on_event, &approver);
    let waited = async {
        let pid = read_pid(&pid_file).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        pid
    };
    let pid = tokio::select! {
        _ = turn => panic!("the turn waits for the answer"),
        pid = waited => pid,
    };
    assert!(all_dead(&[pid]).await, "the CLI must not outlive the cancelled turn");
    let _ = std::fs::remove_dir_all(&dir);
}

// ----------------------------------------------- reading on while a question waits
