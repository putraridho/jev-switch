//! Multi-provider backend layer. A turn runs one CLI (`claude`, `codex`, ...) as a
//! child process that streams JSON lines on stdout. Everything the CLIs share, the
//! spawn, stdin hand-off, line reading, idle watchdog, stderr capture and error
//! mapping, lives once in [`run_turn`]; a [`Backend`] only says how to build its
//! command, how to read one output line, and how to classify its error text.

use std::collections::VecDeque;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::process::Child;
use tokio::task::JoinHandle;

use super::output_pump::{Ended, Idle, Pump};
use super::process_group::GroupKill;
use super::stderr_tail::{collect_stderr, last_chars, STDERR_SHOWN};
use super::stdin_feed::StdinFeed;
use crate::backend::application::ports::{Approver, Backend, NoApprover, Steering};
use crate::backend::domain::error_kind::ErrorKind;
use crate::backend::domain::event::Event;
use crate::backend::domain::outcome_judge::{empty_after_denial, judge};
use crate::backend::domain::turn_error::TurnError;
use crate::backend::domain::turn_outcome::TurnOutcome;
use crate::backend::domain::turn_request::TurnRequest;

/// After the CLI has answered or died, how long to wait for it to be reaped.
const REAP_WAIT: Duration = Duration::from_secs(2);

/// Wait for the child to be reaped, killing it if it will not exit.
async fn reap(child: &mut Child) -> Option<std::process::ExitStatus> {
    match tokio::time::timeout(REAP_WAIT, child.wait()).await {
        Ok(status) => status.ok(),
        Err(_) => {
            let _ = child.start_kill();
            None
        }
    }
}

/// A stream that ended with no result: the stderr tail is the best explanation.
async fn explain_eof(child: &mut Child, stderr: JoinHandle<String>) -> String {
    let status = reap(child).await;
    let tail = tokio::time::timeout(REAP_WAIT, stderr).await.ok().and_then(Result::ok).unwrap_or_default();
    let tail = last_chars(&tail, STDERR_SHOWN);
    if !tail.is_empty() {
        return tail;
    }
    match status {
        Some(s) => format!("the CLI exited ({s}) without a result"),
        None => "the CLI ended without a result".into(),
    }
}

/// Run one turn through `backend` and return its answer or a classified error.
/// `on_event` sees `Tool`, `Text` and `Plan` events while the turn runs. The child
/// is killed if the future is dropped, and after `idle_timeout` of silence.
pub async fn run_turn(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
) -> Result<TurnOutcome, TurnError> {
    run_turn_with(backend, req, idle_timeout, on_event, &NoApprover).await
}

/// [`run_turn`] with someone to answer the tool approvals of an [`Backend::interactive`]
/// CLI. For any other backend `approver` is never asked.
pub async fn run_turn_with(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
) -> Result<TurnOutcome, TurnError> {
    run_turn_steered(backend, req, idle_timeout, on_event, approver, None).await
}

/// [`run_turn_with`] that also takes messages the person types while the turn runs
/// (`steer`), for a backend that [`Backend::steerable`]. A message is written to the CLI as
/// it arrives and [`Event::Steered`] reports when the model has it. One that was not picked up
/// by the time the answer came is simply not reported: whoever sent it still has it.
pub async fn run_turn_steered(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
    steer: Option<Steering<'_>>,
) -> Result<TurnOutcome, TurnError> {
    // A guard, not a call after the await: a cancelled turn drops this future, and the
    // backend's files must go then too.
    let _cleanup = CleanupOnDrop(backend);
    run_child(backend, req, idle_timeout, on_event, approver, steer).await
}

/// Runs [`Backend::cleanup`] when the turn ends or its future is dropped.
struct CleanupOnDrop<'a>(&'a dyn Backend);

impl Drop for CleanupOnDrop<'_> {
    fn drop(&mut self) {
        self.0.cleanup();
    }
}

async fn run_child(
    backend: &dyn Backend,
    req: &TurnRequest,
    idle_timeout: Duration,
    on_event: &mut (dyn FnMut(&Event) + Send),
    approver: &dyn Approver,
    steer: Option<Steering<'_>>,
) -> Result<TurnOutcome, TurnError> {
    let mut cmd = backend.command(req);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // Its own process group, so the whole tree can be killed (see [`GroupKill`]).
    #[cfg(unix)]
    cmd.process_group(0);
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let other = |m: String| TurnError::new(ErrorKind::Other, m);
    let mut child = cmd.spawn().map_err(|e| other(format!("failed to run `{program}` (is it installed?): {e}")))?;
    let mut group = GroupKill(if cfg!(unix) { child.id() } else { None });
    let stdout = child.stdout.take().ok_or_else(|| other(format!("`{program}` gave no stdout")))?;
    let mut feed = StdinFeed::start(child.stdin.take(), backend.stdin_payload(req), backend.interactive());
    let stderr = collect_stderr(child.stderr.take());

    let mut plan = None;
    let mut pump = Pump {
        backend,
        reader: BufReader::new(stdout),
        raw: Vec::new(),
        idle: idle_timeout,
        on_event,
        plan: &mut plan,
        approver,
        feed: &mut feed,
        finished: None,
        queue: VecDeque::new(),
        steer: steer.filter(|_| backend.steerable()),
        sent: VecDeque::new(),
        denied_unasked: Vec::new(),
    };
    let ended = pump.run().await;
    // Messages written to the CLI that it never took before the answer: they are still the
    // person's to run, so the CLI must not start on them now (it would read them from the pipe
    // and the next turn would run them a second time).
    let not_taken = !pump.sent.is_empty();
    let denied_unasked = std::mem::take(&mut pump.denied_unasked);
    drop(pump);
    let classified = |message: String| TurnError::new(backend.classify_error(&message), message);
    let result = match ended {
        Err(Idle) => {
            let _ = child.start_kill();
            let secs = idle_timeout.as_secs_f32();
            Err(TurnError::new(ErrorKind::Stalled, format!("`{program}` printed nothing for {secs}s; stopped it")))
        }
        Ok(Ended::Failed(message)) => {
            let _ = child.start_kill();
            Err(classified(message))
        }
        Ok(Ended::Finished(finish)) if not_taken => {
            // Dropping `group` kills everything the CLI started as well.
            let _ = child.start_kill();
            let _ = reap(&mut child).await;
            judge(finish, plan)
        }
        Ok(Ended::Finished(finish)) => {
            if reap(&mut child).await.is_some() {
                group.disarm();
            }
            judge(finish, plan)
        }
        Ok(Ended::Eof) => Err(classified(explain_eof(&mut child, stderr).await)),
    };
    result.map_err(|e| empty_after_denial(e, &denied_unasked))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::*;
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    
    use std::sync::Mutex;
    use std::time::Duration;
    
    use tokio::process::Command;
    use tokio::sync::mpsc;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-prompt-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
    use std::sync::Arc;

    #[cfg(unix)]
    use super::super::process_group::process_alive as alive;

    /// A backend whose "CLI" is a shell script printing canned lines, so the runner
    /// is tested without any real CLI. Line grammar: `FIN:text`, `FAIL:message`,
    /// `TOOL:summary`, `TEXT:text`.
    fn fake_cli(script: &'static str) -> crate::backend::testing::ScriptedCli {
        crate::backend::testing::ScriptedCli::new(ProviderId::Claude, script)
    }

    fn req() -> TurnRequest {
        TurnRequest { dir: std::env::temp_dir(), model: "m".into(), prompt: "hello".into(), images: vec![], session_id: None, mode: Default::default(), trust_project_settings: false }
    }

    async fn run(script: &'static str, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
        let mut seen = Vec::new();
        let mut on_event = |e: &Event| seen.push(e.clone());
        let r = run_turn(&fake_cli(script), &req(), Duration::from_millis(idle_ms), &mut on_event).await;
        (r, seen)
    }

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
    async fn silent_child_is_killed_as_stalled() {
        let started = std::time::Instant::now();
        let (r, _) = run("echo TEXT:start; exec sleep 30", 300).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// A backend whose script is built at run time (it needs a scratch path).
    struct Owned(String);

    impl Backend for Owned {
        fn provider(&self) -> ProviderId {
            ProviderId::Claude
        }
        fn command(&self, req: &TurnRequest) -> Command {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&self.0).current_dir(&req.dir);
            c
        }
        fn parse_line(&self, line: &str) -> Vec<Event> {
            match line.trim_end().split_once(':') {
                Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                _ => vec![],
            }
        }
        fn classify_error(&self, _: &str) -> ErrorKind {
            ErrorKind::Other
        }
    }

    #[cfg(unix)]
    async fn read_pid(path: &Path) -> i32 {
        for _ in 0..200 {
            if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()) {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{path:?} never written");
    }

    #[cfg(unix)]
    async fn all_dead(pids: &[i32]) -> bool {
        for _ in 0..200 {
            if pids.iter().all(|p| !alive(*p)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    /// A shell that starts a sleeping grandchild and waits for it; both pids go to files.
    #[cfg(unix)]
    fn shell_with_grandchild(dir: &Path) -> (String, PathBuf, PathBuf) {
        std::fs::create_dir_all(dir).unwrap();
        let (shell, child) = (dir.join("shell.pid"), dir.join("child.pid"));
        let script = format!("sleep 60 & echo $! > {c}; echo $$ > {s}; wait", c = child.display(), s = shell.display());
        (script, shell, child)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_turn_kills_the_cli_and_its_children() {
        let dir = scratch_dir("group-drop");
        let (script, shell_file, child_file) = shell_with_grandchild(&dir);
        let backend = Owned(script);
        let request = req();
        let mut on_event = |_: &Event| {};
        let turn = run_turn(&backend, &request, Duration::from_secs(60), &mut on_event);
        let pids = async { (read_pid(&shell_file).await, read_pid(&child_file).await) };
        let (shell, child) = tokio::select! {
            _ = turn => panic!("the turn cannot end by itself"),
            pids = pids => pids,
        };
        // The select dropped the turn: the shell and the sleep it started must both go.
        assert!(all_dead(&[shell, child]).await, "shell alive: {}, grandchild alive: {}", alive(shell), alive(child));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stalled_cli_takes_its_children_with_it() {
        let dir = scratch_dir("group-stall");
        let (script, shell_file, child_file) = shell_with_grandchild(&dir);
        let request = req();
        let r = run_turn(&Owned(script), &request, Duration::from_millis(300), &mut |_| {}).await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Stalled);
        let pids = [read_pid(&shell_file).await, read_pid(&child_file).await];
        assert!(all_dead(&pids).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_cli_takes_its_children_with_it() {
        let dir = scratch_dir("group-fail");
        std::fs::create_dir_all(&dir).unwrap();
        let child_file = dir.join("child.pid");
        let script = format!("sleep 60 & echo $! > {}; echo 'FAIL:boom'; wait", child_file.display());
        struct Fails(Owned);
        impl Backend for Fails {
            fn provider(&self) -> ProviderId {
                ProviderId::Claude
            }
            fn command(&self, req: &TurnRequest) -> Command {
                self.0.command(req)
            }
            fn parse_line(&self, line: &str) -> Vec<Event> {
                match line.trim_end().split_once(':') {
                    Some(("FAIL", m)) => vec![Event::Failed { message: m.into() }],
                    _ => vec![],
                }
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
        }
        let request = req();
        let r = run_turn(&Fails(Owned(script)), &request, Duration::from_secs(5), &mut |_| {}).await;
        assert!(r.is_err());
        assert!(all_dead(&[read_pid(&child_file).await]).await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_clean_finish_leaves_background_work_the_agent_started_alone() {
        let dir = scratch_dir("group-clean");
        std::fs::create_dir_all(&dir).unwrap();
        let child_file = dir.join("child.pid");
        let script = format!("sleep 30 >/dev/null 2>&1 & echo $! > {}; echo FIN:ok", child_file.display());
        let request = req();
        let r = run_turn(&Owned(script), &request, Duration::from_secs(5), &mut |_| {}).await;
        assert_eq!(r.unwrap().text, "ok");
        let child = read_pid(&child_file).await;
        assert!(alive(child), "a finished turn does not kill its group");
        // SAFETY: tidying our own test process.
        unsafe { libc::kill(child, libc::SIGKILL) };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_cli_that_lingers_after_its_answer_does_not_hold_the_turn_for_the_watchdog() {
        // The answer, then a CLI that keeps stdout open and stays silent (a background monitor).
        let request = req();
        let started = std::time::Instant::now();
        let r = run_turn(&Owned("echo FIN:ok; sleep 30".into()), &request, Duration::from_secs(60), &mut |_| {}).await;
        assert_eq!(r.unwrap().text, "ok");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    }



    #[tokio::test]
    async fn a_dropped_turn_still_cleans_up_the_backends_files() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Flag(Arc<AtomicBool>);
        impl Backend for Flag {
            fn provider(&self) -> ProviderId {
                ProviderId::Claude
            }
            fn command(&self, req: &TurnRequest) -> Command {
                let mut c = Command::new("sh");
                c.arg("-c").arg("sleep 30").current_dir(&req.dir);
                c
            }
            fn parse_line(&self, _: &str) -> Vec<Event> {
                vec![]
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
            fn cleanup(&self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cleaned = Arc::new(AtomicBool::new(false));
        let request = req();
        let backend = Flag(cleaned.clone());
        let gone = tokio::time::timeout(Duration::from_millis(100), run_turn(&backend, &request, Duration::from_secs(60), &mut |_| {})).await;
        assert!(gone.is_err(), "the turn was still running when it was dropped");
        assert!(cleaned.load(Ordering::SeqCst));
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
                Some(("ASK", t)) => vec![Event::PermissionRequest(PermissionRequest { id: "q1".into(), tool: "Bash".into(), summary: t.into(), input: serde_json::json!({}), blocked_path: None })],
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

    #[tokio::test]
    async fn run_turn_cleans_up_the_backends_files_after_every_kind_of_turn() {
        struct Writes {
            files: PromptFiles,
            path: Mutex<Option<PathBuf>>,
            script: &'static str,
        }
        impl Backend for Writes {
            fn provider(&self) -> ProviderId {
                ProviderId::Grok
            }
            fn command(&self, req: &TurnRequest) -> Command {
                *self.path.lock().unwrap() = self.files.arg(&req.prompt).file().map(Path::to_path_buf);
                let mut c = Command::new("sh");
                c.arg("-c").arg(self.script).current_dir(&req.dir);
                c
            }
            fn parse_line(&self, line: &str) -> Vec<Event> {
                match line.trim_end().split_once(':') {
                    Some(("FIN", t)) => vec![Event::Finished(Finish { text: t.into(), session_id: None, usage: Usage::default() })],
                    _ => vec![],
                }
            }
            fn classify_error(&self, _: &str) -> ErrorKind {
                ErrorKind::Other
            }
            fn cleanup(&self) {
                self.files.cleanup();
            }
        }
        let dir = scratch_dir("runner");
        let mut request = req();
        request.prompt = "p".repeat(MAX_PROMPT_ARG_BYTES + 1);
        for script in ["echo FIN:ok", "exit 1"] {
            let b = Writes { files: PromptFiles::in_dir(dir.clone()), path: Mutex::new(None), script };
            let r = run_turn(&b, &request, Duration::from_secs(5), &mut |_| {}).await;
            assert_eq!(r.is_ok(), script.contains("FIN"));
            let path = b.path.lock().unwrap().clone().expect("the long prompt was written");
            assert!(!path.exists(), "{script}: left {path:?} behind");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }



    // ------------------------------------------------------------ host approvals

    /// An interactive "CLI": a shell script that asks questions and reads the answers from
    /// stdin. Output grammar: `ASK:<tool>:<summary>` (id `q1`), `ASKID:<id>:<tool>:<summary>`,
    /// `CANCEL:<id>`, `TEXT:<t>`, `TOOL:<t>`, `FIN:<text>`. The answers it receives are
    /// `<id>:ALLOW` (also for always) and `<id>:DENY:<reason>`.
    struct Asking(String);

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
                    input: serde_json::json!({ "command": summary }),
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

    const ASK_AND_REPORT: &str =
        r#"read prompt; echo "ASK:Bash:cargo test --quiet"; read ans; case "$ans" in q1:ALLOW) echo "FIN:ran";; *) echo "FIN:refused ${ans#q1:DENY:}";; esac"#;

    /// An approver that answers after `wait`, and remembers what it was asked, what it was
    /// told was withdrawn, and (in `log`) what happened in which order.
    struct Scripted {
        wait: Duration,
        decision: Decision,
        asked: Mutex<Vec<(String, String)>>,
        withdrawn: Mutex<Vec<String>>,
        log: Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(wait_ms: u64, decision: Decision) -> Self {
            Self {
                wait: Duration::from_millis(wait_ms),
                decision,
                asked: Mutex::new(Vec::new()),
                withdrawn: Mutex::new(Vec::new()),
                log: Mutex::new(Vec::new()),
            }
        }

        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }

        fn withdrawn_ids(&self) -> Vec<String> {
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

    async fn run_asking(script: &str, approver: &dyn Approver, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
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
}
