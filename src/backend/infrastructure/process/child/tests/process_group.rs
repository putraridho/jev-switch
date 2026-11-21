use super::*;
use std::sync::Arc;
use tokio::process::Command;

#[cfg(unix)]
use super::super::super::process_group::process_alive as alive;

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
pub(super) async fn read_pid(path: &Path) -> i32 {
    for _ in 0..200 {
        if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()) {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{path:?} never written");
}

#[cfg(unix)]
pub(super) async fn all_dead(pids: &[i32]) -> bool {
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
