use super::*;
use tokio::process::Command;

#[tokio::test]
async fn finished_turn_streams_events_and_returns_outcome() {
    let (r, seen) = run("echo noise; echo TOOL:Bash ls; echo TEXT:hi; echo FIN:done", 5000).await;
    let out = r.unwrap();
    assert_eq!(out.text, "done");
    assert_eq!(out.session_id.as_deref(), Some("s1"));
    assert_eq!(out.usage.output_tokens, 7);
    assert_eq!(seen, vec![Event::Tool("Bash ls".into()), Event::Text("hi".into())]);
}

#[tokio::test]
async fn prompt_is_delivered_on_stdin_and_stdin_is_closed() {
    // `cat` only returns once stdin is closed, so this also proves the close.
    let (r, _) = run("echo FIN:$(cat)", 5000).await;
    assert_eq!(r.unwrap().text, "hello");
}

#[tokio::test]
async fn failed_event_is_classified() {
    let (r, _) = run("echo 'FAIL:You have hit your usage limit, resets 5pm'", 5000).await;
    let e = r.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Quota { resets_at: Some("5pm".into()) });
}

#[tokio::test]
async fn failure_logging_uses_the_command_override_and_can_be_disabled() {
    struct LoggedCli {
        backend: crate::testing::ScriptedCli,
        log: String,
    }
    impl Backend for LoggedCli {
        fn provider(&self) -> ProviderId {
            self.backend.provider()
        }
        fn command(&self, request: &TurnRequest) -> Command {
            let mut command = self.backend.command(request);
            command.env("JEV_FAILURE_LOG", &self.log);
            command
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            self.backend.parse_line(line)
        }
        fn classify_error(&self, message: &str) -> ErrorKind {
            self.backend.classify_error(message)
        }
    }
    let dir = jev_testkit::temp_folder::Dir::new();
    let path = dir.path().join("failures.jsonl");
    let mut backend = LoggedCli { backend: fake_cli("echo FAIL:boom"), log: String::new() };
    let result = run_turn(&backend, &req(), Duration::from_secs(1), &mut |_| {}).await;
    assert_eq!(result.unwrap_err().message, "boom");
    assert!(!path.exists());

    backend.log = path.to_string_lossy().into_owned();
    let result = run_turn(&backend, &req(), Duration::from_secs(1), &mut |_| {}).await;
    assert_eq!(result.unwrap_err().message, "boom");
    let rows = std::fs::read_to_string(&path).unwrap();
    assert_eq!(rows.lines().count(), 1);
    let row: serde_json::Value = serde_json::from_str(rows.trim()).unwrap();
    assert_eq!(row["message"], "boom");
    assert_eq!(row["raw"], "FAIL:boom");
}

#[tokio::test]
async fn silent_child_is_killed_as_stalled() {
    let started = std::time::Instant::now();
    let (r, _) = run("echo TEXT:start; exec sleep 30", 300).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
    assert!(started.elapsed() < Duration::from_secs(10));
}
#[tokio::test]
async fn empty_answer_is_an_error() {
    let (r, _) = run("echo 'FIN:   '", 5000).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Empty);
}

#[tokio::test]
async fn limit_text_as_answer_is_quota() {
    let (r, _) = run("echo \"FIN:You've hit your session limit · resets 5pm (Asia/Jakarta)\"", 5000).await;
    let e = r.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Quota { resets_at: Some("5pm (Asia/Jakarta)".into()) });
    assert!(e.message.contains("usage limit reached"));
}

#[tokio::test]
async fn exit_without_result_uses_stderr_tail() {
    let (r, _) = run("echo 'Error: not logged in' >&2; exit 1", 5000).await;
    let e = r.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert!(e.message.contains("not logged in"));
}

#[tokio::test]
async fn exit_without_result_or_stderr_names_the_exit() {
    let (r, _) = run("exit 3", 5000).await;
    let e = r.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Other);
    assert!(e.message.contains("without a result"));
}

#[tokio::test]
async fn stderr_tail_is_trimmed_to_the_last_400_chars() {
    let (r, _) = run("head -c 5000 /dev/zero | tr '\\0' x >&2; echo END >&2", 5000).await;
    let m = r.unwrap_err().message;
    assert!(m.chars().count() <= 400 && m.ends_with("END"));
}

#[tokio::test]
async fn missing_program_is_other() {
    struct Missing;
    impl Backend for Missing {
        fn provider(&self) -> ProviderId {
            ProviderId::Codex
        }
        fn command(&self, _: &TurnRequest) -> Command {
            Command::new("/nonexistent/jev-cli")
        }
        fn parse_line(&self, _: &str) -> Vec<Event> {
            vec![]
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }
    let r = run_turn(&Missing, &req(), Duration::from_secs(1), &mut |_| {}).await;
    assert_eq!(r.unwrap_err().kind, ErrorKind::Other);
}

#[test]
fn provider_ids_round_trip() {
    for p in [ProviderId::Claude, ProviderId::DeepSeek, ProviderId::Codex, ProviderId::Gemini, ProviderId::Grok] {
        assert_eq!(p.as_str().parse::<ProviderId>(), Ok(p));
    }
    assert_eq!("  Codex ".parse::<ProviderId>(), Ok(ProviderId::Codex));
    assert!("openai".parse::<ProviderId>().is_err());
}
