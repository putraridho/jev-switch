use super::*;
use super::approvals::*;

/// Run `script` with every event it produces logged next to the approver's own log.
async fn run_logged(script: &str, approver: &Scripted, idle_ms: u64) -> Result<TurnOutcome, TurnError> {
    let mut on_event = |e: &Event| approver.note(format!("event {e:?}"));
    run_turn_with(&Asking(script.into()), &req(), Duration::from_millis(idle_ms), &mut on_event, approver).await
}

/// The script ends by copying whatever the runner wrote to its stdin into `file`, so a test
/// can see exactly which answers reached the CLI. The runner closes stdin after the result.
fn recording(dir: &Path, body: &str) -> (String, PathBuf) {
    std::fs::create_dir_all(dir).unwrap();
    let file = dir.join("received");
    (format!("{body}; cat > {}", file.display()), file)
}

fn received(file: &Path) -> String {
    std::fs::read_to_string(file).unwrap_or_default()
}

#[tokio::test]
async fn a_withdrawn_question_is_dropped_unanswered_and_the_ui_is_told() {
    let dir = scratch_dir("withdraw");
    // The person never answers (a minute); the CLI withdraws the question, with a progress
    // line before it, and goes on.
    let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "TOOL:Read a"; sleep 0.2; echo "CANCEL:q1"; echo "FIN:done""#);
    let approver = Scripted::new(60_000, Decision::Allow);
    let started = std::time::Instant::now();
    let r = run_logged(&script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "done");
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert_eq!(approver.withdrawn_ids(), ["q1"]);
    assert_eq!(received(&file), "", "no answer is written for a withdrawn question");
    // The event that arrived while the question was pending still reaches the UI, after it.
    assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "withdrawn q1", r#"event Tool("Read a")"#]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn events_arriving_while_a_question_waits_are_delivered_in_order_after_it() {
    let approver = Scripted::new(400, Decision::Allow);
    let script = r#"read p; echo "ASK:Bash:x"; echo "TEXT:one"; echo "TOOL:two"; echo "TEXT:three"; read a; echo "TOOL:four"; echo "FIN:$a""#;
    let r = run_logged(script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "q1:ALLOW");
    assert_eq!(
        *approver.log.lock().unwrap(),
        ["asked q1", "decided q1", r#"event Text("one")"#, r#"event Tool("two")"#, r#"event Text("three")"#, r#"event Tool("four")"#]
    );
}

#[tokio::test]
async fn the_cli_ending_while_a_question_waits_drops_the_question() {
    let approver = Scripted::new(60_000, Decision::Allow);
    let started = std::time::Instant::now();
    let r = run_logged(r#"read p; echo "ASK:Bash:x"; echo "TEXT:last words"; exit 3"#, &approver, 5000).await;
    let err = r.unwrap_err();
    assert!(err.message.contains("exited"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(approver.withdrawn_ids(), ["q1"]);
    assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "withdrawn q1", r#"event Text("last words")"#]);
}

#[tokio::test]
async fn a_result_while_a_question_waits_drops_the_question_and_keeps_the_answer() {
    let dir = scratch_dir("finish-pending");
    let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "FIN:already done""#);
    let approver = Scripted::new(60_000, Decision::Allow);
    let r = run_logged(&script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "already done");
    assert_eq!(approver.withdrawn_ids(), ["q1"]);
    assert_eq!(received(&file), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_idle_watchdog_runs_again_once_a_question_was_withdrawn() {
    let approver = Scripted::new(60_000, Decision::Allow);
    let r = run_logged(r#"read p; echo "ASK:Bash:x"; sleep 0.1; echo "CANCEL:q1"; exec sleep 30"#, &approver, 400).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
    assert_eq!(approver.withdrawn_ids(), ["q1"]);
}

#[tokio::test]
async fn two_questions_one_after_the_other_are_each_answered_once_with_their_own_id() {
    let dir = scratch_dir("sequential");
    let (script, file) = recording(
        &dir,
        r#"read p; echo "ASKID:a:Bash:one"; read r1; echo "ASKID:b:Bash:two"; read r2; echo "FIN:$r1|$r2""#,
    );
    let approver = Scripted::new(30, Decision::Allow);
    let r = run_logged(&script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "a:ALLOW|b:ALLOW");
    assert_eq!(received(&file), "");
    assert!(approver.withdrawn_ids().is_empty());
    assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a", "asked b", "decided b"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn questions_asked_together_are_decided_one_at_a_time_in_order() {
    let approver = Scripted::new(100, Decision::Deny { reason: "no".into() });
    let script = r#"read p; echo "ASKID:a:Bash:one"; echo "ASKID:b:Bash:two"; read r1; read r2; echo "FIN:$r1|$r2""#;
    let r = run_logged(script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "a:DENY:no|b:DENY:no");
    assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a", "asked b", "decided b"]);
}

#[tokio::test]
async fn a_waiting_question_that_is_withdrawn_is_never_asked() {
    let approver = Scripted::new(300, Decision::Allow);
    let script = r#"read p; echo "ASKID:a:Bash:one"; echo "ASKID:b:Bash:two"; echo "CANCEL:b"; read r1; echo "FIN:$r1""#;
    let r = run_logged(script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "a:ALLOW");
    assert_eq!(*approver.log.lock().unwrap(), ["asked a", "decided a"], "b was withdrawn before its turn");
    assert!(approver.withdrawn_ids().is_empty(), "it was never shown, so there is nothing to take down");
}

#[tokio::test]
async fn a_cancel_for_another_id_changes_nothing() {
    let dir = scratch_dir("other-id");
    let (script, file) = recording(&dir, r#"read p; echo "ASK:Bash:x"; echo "CANCEL:zzz"; read a; echo "FIN:$a""#);
    let approver = Scripted::new(200, Decision::Allow);
    let r = run_logged(&script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "q1:ALLOW", "the question stayed and was answered");
    assert!(approver.withdrawn_ids().is_empty());
    assert_eq!(*approver.log.lock().unwrap(), ["asked q1", "decided q1"]);
    assert_eq!(received(&file), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_cancel_after_the_answer_is_ignored() {
    let approver = Scripted::new(0, Decision::Allow);
    let script = r#"read p; echo "ASK:Bash:x"; read a; echo "CANCEL:q1"; echo "FIN:$a""#;
    let r = run_logged(script, &approver, 5000).await;
    assert_eq!(r.unwrap().text, "q1:ALLOW");
    assert!(approver.withdrawn_ids().is_empty());
}
