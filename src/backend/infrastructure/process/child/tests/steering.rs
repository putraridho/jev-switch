use super::*;
use std::future::Future;
use std::pin::Pin;
use tokio::process::Command;
use tokio::sync::mpsc;

/// An interactive "CLI" that takes messages on stdin: the prompt is its first line, each
/// later line `MSG:<text>` is echoed back as `ECHO:<text>` after a pause (the model taking
/// it at its next step), and `GO` on a line ends the turn.
struct Steerable(&'static str);

impl Backend for Steerable {
    fn provider(&self) -> ProviderId {
        ProviderId::Claude
    }
    fn command(&self, req: &TurnRequest) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(self.0).current_dir(&req.dir);
        c
    }
    fn parse_line(&self, line: &str) -> Vec<Event> {
        let line = line.trim_end();
        match line.split_once(':') {
            Some(("ECHO", t)) => vec![Event::Steered(t.into())],
            Some(("ASK", t)) => vec![Event::PermissionRequest(PermissionRequest { id: "q1".into(), tool: "Bash".into(), summary: t.into(), input: crate::approval::infrastructure::tool_input_wire::tool_input_from_json(&serde_json::json!({})), blocked_path: None })],
            Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
            _ => vec![],
        }
    }
    fn interactive(&self) -> bool {
        true
    }
    fn steerable(&self) -> bool {
        true
    }
    fn steer_line(&self, text: &str) -> Option<String> {
        Some(format!("MSG:{text}"))
    }
    fn stdin_payload(&self, req: &TurnRequest) -> Option<String> {
        Some(format!("{}\n", req.prompt))
    }
    fn classify_error(&self, _: &str) -> ErrorKind {
        ErrorKind::Other
    }
}

async fn steered_turn(script: &'static str, send: &[&str], prepare: &(dyn Fn(&str) -> Option<String> + Send + Sync)) -> (Result<TurnOutcome, TurnError>, Vec<String>) {
    let (tx, rx) = mpsc::unbounded_channel();
    for text in send {
        tx.send((*text).to_string()).unwrap();
    }
    let mut taken = Vec::new();
    let result = {
        let mut on_event = |e: &Event| {
            if let Event::Steered(t) = e {
                taken.push(t.clone());
            }
        };
        run_turn_steered(&Steerable(script), &req(), Duration::from_secs(10), &mut on_event, &NoApprover, Some(Steering { rx, prepare })).await
    };
    drop(tx);
    (result, taken)
}

/// Reads the prompt, then each message: `MSG:x` is echoed as `ECHO:x`, `MSG:GO` ends the turn.
const ECHO_ALL: &str = "read prompt; while read line; do case \"$line\" in MSG:GO) echo FIN:done; exit 0;; MSG:*) echo \"ECHO:${line#MSG:}\";; esac; done";

#[tokio::test]
async fn a_message_typed_during_the_turn_is_sent_in_and_reported_when_taken() {
    // `GO` ends the turn, so the first message is echoed before the answer comes.
    let (result, taken) = steered_turn(ECHO_ALL, &["also run the tests", "GO"], &|t| Some(t.to_string())).await;
    assert_eq!(result.unwrap().text, "done");
    assert_eq!(taken, vec!["also run the tests".to_string()], "only the message that was sent in is reported, as typed");
}

#[tokio::test]
async fn a_message_the_policy_refuses_is_not_sent_and_not_reported() {
    let prepare = |t: &str| (!t.contains("secret")).then(|| t.to_string());
    let (result, taken) = steered_turn(ECHO_ALL, &["use the secret file", "GO"], &prepare).await;
    assert_eq!(result.unwrap().text, "done");
    assert!(taken.is_empty(), "{taken:?}");
}

#[tokio::test]
async fn the_text_sent_may_differ_from_the_text_typed_and_the_typed_one_is_reported() {
    let prepare = |t: &str| Some(t.replace("sk-123", "[hidden]"));
    let (result, taken) = steered_turn(ECHO_ALL, &["use key sk-123", "GO"], &prepare).await;
    assert!(result.is_ok());
    assert_eq!(taken, vec!["use key sk-123".to_string()], "the person's own words come back, so their queue can match them");
}

#[tokio::test]
async fn a_message_not_taken_before_the_answer_is_never_reported() {
    // The CLI answers without reading what was sent: it is still the person's to run next.
    let (result, taken) = steered_turn("read prompt; echo FIN:done", &["too late"], &|t| Some(t.to_string())).await;
    assert_eq!(result.unwrap().text, "done");
    assert!(taken.is_empty(), "{taken:?}");
}

#[tokio::test]
async fn the_prompts_own_echo_is_not_a_steered_message() {
    // A CLI that echoes every user message, the first prompt included.
    let script = "read prompt; echo \"ECHO:$prompt\"; echo FIN:done";
    let (result, taken) = steered_turn(script, &[], &|t| Some(t.to_string())).await;
    assert!(result.is_ok());
    assert!(taken.is_empty(), "{taken:?}");
}

#[tokio::test]
async fn a_message_the_cli_never_echoes_does_not_hide_the_ones_after_it() {
    // Takes `b` into the turn but never reports `a`.
    let script = "read prompt; while read line; do case \"$line\" in MSG:GO) echo FIN:done; exit 0;; MSG:b) echo ECHO:b;; esac; done";
    let (result, taken) = steered_turn(script, &["a", "b", "GO"], &|t| Some(t.to_string())).await;
    assert!(result.is_ok());
    assert_eq!(taken, vec!["b".to_string()]);
}

#[cfg(unix)]
#[tokio::test]
async fn the_cli_is_stopped_at_once_when_a_message_was_not_taken_so_it_runs_only_once_later() {
    let mark = std::env::temp_dir().join(format!("jev-steer-mark-{}", std::process::id()));
    let _ = std::fs::remove_file(&mark);
    // Answers, then (after the time a model needs to start) would read the message left in the pipe and act on it.
    let script: &'static str = Box::leak(format!("read prompt; sleep 0.3; echo FIN:done; sleep 1.5; read line; touch {}", mark.display()).into_boxed_str());
    let (result, taken) = steered_turn(script, &["run the tests"], &|t| Some(t.to_string())).await;
    assert_eq!(result.unwrap().text, "done");
    assert!(taken.is_empty());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let acted = mark.exists();
    let _ = std::fs::remove_file(&mark);
    assert!(!acted, "the CLI acted on a message that stays in the person's queue");
}

/// An approver that never answers: the question stays open until the CLI ends it.
#[cfg(unix)]
struct NeverAnswers;

#[cfg(unix)]
impl Approver for NeverAnswers {
    fn decide<'a>(&'a self, _: &'a PermissionRequest) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(std::future::pending())
    }
}

#[cfg(unix)]
#[tokio::test]
async fn an_answer_that_comes_while_a_question_is_open_also_stops_the_cli_at_once() {
    let mark = std::env::temp_dir().join(format!("jev-steer-ask-{}", std::process::id()));
    let _ = std::fs::remove_file(&mark);
    let script: &'static str = Box::leak(format!("read prompt; echo ASK:ls; sleep 0.4; echo FIN:done; sleep 1.5; read line; touch {}", mark.display()).into_boxed_str());
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send("also run clippy".to_string()).unwrap();
    let mut on_event = |_: &Event| {};
    let prepare = |t: &str| Some(t.to_string());
    let out = run_turn_steered(&Steerable(script), &req(), Duration::from_secs(10), &mut on_event, &NeverAnswers, Some(Steering { rx, prepare: &prepare })).await;
    assert_eq!(out.unwrap().text, "done");
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let acted = mark.exists();
    let _ = std::fs::remove_file(&mark);
    assert!(!acted, "the CLI got time to act on a message that stays in the person's queue");
}

#[tokio::test]
async fn typing_does_not_keep_a_silent_cli_alive() {
    let (tx, rx) = mpsc::unbounded_channel();
    let feeder = tokio::spawn(async move {
        for i in 0..40 {
            if tx.send(format!("message {i}")).is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let mut on_event = |_: &Event| {};
    let prepare = |t: &str| Some(t.to_string());
    let started = std::time::Instant::now();
    let out = run_turn_steered(&Steerable("read prompt; sleep 30"), &req(), Duration::from_millis(700), &mut on_event, &NoApprover, Some(Steering { rx, prepare: &prepare })).await;
    feeder.abort();
    assert!(matches!(out, Err(ref e) if e.kind == ErrorKind::Stalled), "{out:?}");
    assert!(started.elapsed() < Duration::from_millis(3500), "typing kept it waiting: {:?}", started.elapsed());
}

#[tokio::test]
async fn a_backend_that_cannot_take_messages_ignores_the_channel() {
    let (tx, rx) = mpsc::unbounded_channel();
    tx.send("hello".to_string()).unwrap();
    let mut on_event = |_: &Event| {};
    let prepare = |t: &str| Some(t.to_string());
    let out = run_turn_steered(&fake_cli("echo FIN:ok"), &req(), Duration::from_secs(5), &mut on_event, &NoApprover, Some(Steering { rx, prepare: &prepare })).await;
    assert_eq!(out.unwrap().text, "ok");
}
