use super::*;
use crate::approval::application::extension_hooks::runner::{kept_text, run_command, refusal_for, Ran, STDERR_LIMIT};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn a_status_2_pre_tool_hook_refuses_with_what_it_said() {
    let dir = temp("refuse");
    let h = hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","tool":"Bash","run":"echo no rm please >&2; exit 2"}]}"#);
    let out = h.run(Event::PreTool, Some("Bash"), Some("rm -rf /"));
    assert_eq!(out.refused.as_deref(), Some("no rm please"));
    // The matcher keeps other tools out of it.
    assert_eq!(h.run(Event::PreTool, Some("Read"), Some("a.txt")), Outcome::default());
    assert_eq!(h.run(Event::PostTool, Some("Bash"), None), Outcome::default());
}

#[test]
fn the_event_arrives_as_json_on_stdin_and_in_the_environment() {
    let dir = temp("payload");
    let run = format!("cat > {0}/stdin.json; echo \"$JEV_HOOK_EVENT:$JEV_HOOK_TOOL\" > {0}/env.txt", dir.display());
    let h = hooks_with(&dir, &serde_json::json!({"hooks":[{"event":"post_tool","run":run}]}).to_string());
    assert_eq!(h.run(Event::PostTool, Some("Edit"), Some("src/a.rs")), Outcome::default());
    let stdin = std::fs::read_to_string(dir.join("stdin.json")).unwrap();
    assert!(stdin.contains(r#""event":"post_tool""#) && stdin.contains(r#""tool":"Edit""#) && stdin.contains("src/a.rs"), "{stdin}");
    assert_eq!(std::fs::read_to_string(dir.join("env.txt")).unwrap().trim(), "post_tool:Edit");
}

#[test]
fn failures_and_timeouts_warn_but_do_not_refuse() {
    let dir = temp("warn");
    let h = hooks_with(&dir, r#"{"hooks":[{"event":"turn_end","run":"exit 3"},{"event":"turn_end","run":"sleep 5","timeout_secs":0},{"event":"post_tool","run":"exit 2"}]}"#);
    let out = h.run(Event::TurnEnd, None, None);
    assert_eq!(out.refused, None);
    assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
    assert!(out.warnings[0].contains("status 3") && out.warnings[1].contains("was stopped"));
    // Status 2 only refuses before a tool call.
    let post = h.run(Event::PostTool, Some("Bash"), None);
    assert_eq!((post.refused, post.warnings.len()), (None, 1));
}

#[test]
fn a_hook_that_leaves_a_background_process_does_not_hold_the_turn() {
    let dir = temp("background");
    let h = hooks_with(&dir, r#"{"hooks":[{"event":"turn_end","run":"sleep 5 &"}]}"#);
    let started = Instant::now();
    let out = h.run(Event::TurnEnd, None, None);
    assert_eq!(out, Outcome::default());
    assert!(started.elapsed() < Duration::from_secs(3), "waited {:?} for a background process", started.elapsed());
}

#[test]
fn a_refusal_keeps_its_reason_when_a_background_process_holds_stderr() {
    let dir = temp("background-refusal");
    let h = hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","run":"echo no rm please >&2; sleep 5 & exit 2"}]}"#);
    let started = Instant::now();
    let out = h.run(Event::PreTool, Some("Bash"), Some("rm -rf /"));
    assert!(out.refused.as_deref().is_some_and(|r| r.contains("no rm please")), "{out:?}");
    assert!(started.elapsed() < Duration::from_secs(3), "waited {:?}", started.elapsed());
}

#[test]
fn long_stderr_is_cut_at_a_character_and_marked() {
    assert_eq!(kept_text(b"short"), "short");
    // One ASCII byte first, so the limit (even) falls inside a two-byte `é`.
    let mut bytes = format!("x{}", "é".repeat(STDERR_LIMIT)).into_bytes();
    bytes.truncate(STDERR_LIMIT + 1);
    let kept = kept_text(&bytes);
    assert!(kept.ends_with("é…(cut)") && !kept.contains('\u{FFFD}'));
    // A bad byte the hook wrote right at the limit is kept (shown as U+FFFD).
    let mut bad = vec![b'a'; STDERR_LIMIT + 1];
    bad[STDERR_LIMIT - 1] = 0xFF;
    assert!(kept_text(&bad).ends_with("\u{FFFD}…(cut)"));
}

#[test]
fn a_timeout_stops_what_the_hook_started() {
    let dir = temp("tree");
    let marker = dir.join("survived");
    let cmd = format!("(sleep 1; touch {}) & sleep 5", marker.display());
    let ran = run_command(&cmd, &dir, "", Event::TurnEnd, None, Duration::from_millis(200));
    assert!(matches!(ran, Ran::TimedOut));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "a process the hook started outlived its timeout");
}

#[test]
fn a_hook_that_never_reads_a_large_input_still_times_out() {
    let dir = temp("unread-input");
    let input = "x".repeat(1 << 20);
    let started = Instant::now();
    let ran = run_command("sleep 5", &dir, &input, Event::PostTool, None, Duration::from_millis(300));
    assert!(matches!(ran, Ran::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(3), "waited {:?} on the input pipe", started.elapsed());
}

#[tokio::test]
async fn the_async_helper_returns_the_refusal() {
    let dir = temp("async");
    let h = Arc::new(hooks_with(&dir, r#"{"hooks":[{"event":"pre_tool","run":"exit 2"}]}"#));
    let out = refusal_for(&h, "Bash", "ls").await.unwrap();
    assert!(out.refused.unwrap().contains("refused by the hook"));
    assert!(refusal_for(&Arc::new(Hooks::none(&dir)), "Bash", "ls").await.is_none());
}
