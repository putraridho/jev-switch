//! Turns: what one prints, stopping with Ctrl-C, `-p`, `--every`, the bell and the pager.

use super::*;
use crate::session::domain::turn_status::TurnStatus;
use super::super::turn::bell_after;
use super::super::pager::page;
use super::super::turn_race::{press_outside_pager, run_turn};

#[tokio::test]
async fn help_status_and_unknown_commands_print_to_stdout() {
    let mut r = rig("info", ANSWER);
    assert_eq!(run(&mut r, "/help").await, Flow::Continue);
    assert!(r.out.text().contains("/model") && r.out.text().contains("Ctrl-C"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/status").await;
    let s = r.out.text();
    assert!(s.contains("Routing") && s.contains("on") && s.contains("claude"), "{s}");
    r.out.clear();
    run(&mut r, "/provders").await;
    let u = r.out.text();
    assert!(u.contains("unknown command /provders") && u.contains("Did you mean /providers?") && u.contains("/help"), "{u}");
    assert_eq!(run(&mut r, "/xyzzy").await, Flow::Continue);
    assert!(!r.out.text().contains("Did you mean /xyzzy"));
    assert_eq!(run(&mut r, "/quit").await, Flow::Quit);
    assert_eq!(r.err.text(), "", "no model was asked and nothing went to stderr");
}

#[tokio::test]
async fn a_turn_prints_route_tools_answer_summary_and_the_review_hint() {
    let mut r = rig("turn", ANSWER);
    let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
    r.repl.finish(&report, elapsed);
    let (o, e) = (r.out.text(), r.err.text());
    assert!(o.contains("working") && o.contains("the answer"), "{o}");
    assert!(e.contains("→ claude") && e.contains("⏺ Edit a.txt"), "{e}");
    assert!(e.contains("✓ claude") && e.contains("1 file") && e.contains("/diff to review, /undo to revert"), "{e}");
    assert!(!e.contains("Answered") && !e.contains("TurnReport"), "no Debug output: {e}");
}

#[tokio::test]
async fn a_failed_turn_ends_in_an_error_block_with_steps_and_a_summary() {
    let mut r = rig("fail", "printf 'FAIL:segmentation fault\\n'");
    let TurnEnd::Done(report, elapsed) = judged_turn(&mut r, "change a").await else { panic!() };
    assert_eq!(report.status, TurnStatus::Failed);
    r.repl.finish(&report, elapsed);
    let e = r.err.text();
    assert!(e.contains("✗ The turn failed") && e.contains("segmentation fault") && e.contains("`jev-code doctor`"), "{e}");
    assert!(e.contains("✗ claude"), "the summary marks the failure: {e}");
    assert_eq!(r.out.text(), "");
}

#[tokio::test]
async fn ctrl_c_stops_the_turn_and_the_session_carries_on() {
    let mut r = rig("ctrlc", "printf 'TOOL:Edit a.txt\\n'; sleep 30; printf 'FIN:late\\n'");
    let mut int = Presses { pending: vec![Duration::from_millis(300)] };
    let TurnEnd::Done(report, _) = run_turn(&mut r.repl.engine, &r.repl.console, "do it", &mut int).await else { panic!("aborted") };
    assert_eq!(report.status, TurnStatus::Cancelled);
    r.repl.finish(&report, Duration::from_secs(3));
    let e = r.err.text();
    assert!(e.contains("■ stopped · 3s"), "{e}");
    assert!(!e.contains("cancelled") && !e.contains("[cancelled]"), "{e}");
    // The same engine takes the next prompt.
    let mut ok = rig("ctrlc-next", ANSWER);
    let mut none = Presses { pending: vec![] };
    let TurnEnd::Done(next, _) = run_turn(&mut ok.repl.engine, &ok.repl.console, "again", &mut none).await else { panic!() };
    assert_eq!(next.status, TurnStatus::Answered);
}

#[tokio::test]
async fn ctrl_c_inside_an_approval_pager_does_not_stop_the_turn() {
    let console = Console::new(Box::new(Buf::default()), Box::new(Buf::default()), ConsoleConfig {
        out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: "/tmp".into(), stream_answers: true,
    });
    console.begin_paging();
    // Two presses: the first while the pager is open, the second after it closed.
    let mut int = Presses { pending: vec![Duration::from_millis(20), Duration::from_millis(200)] };
    let started = Instant::now();
    let closer = console.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        closer.end_paging();
    });
    tokio::time::timeout(Duration::from_secs(5), press_outside_pager(&mut int, &console)).await.expect("the second press counts");
    assert!(started.elapsed() >= Duration::from_millis(200), "the press inside the pager was ignored: {:?}", started.elapsed());
}

#[tokio::test]
async fn a_second_ctrl_c_while_stopping_gives_up_waiting() {
    let mut r = rig("ctrlc2", "printf 'TOOL:Edit a.txt\\n'; sleep 30");
    let mut int = Presses { pending: vec![Duration::from_millis(200), Duration::from_millis(50)] };
    // The engine would stop on the first press; the second arrives while it is stopping
    // only if stopping takes a while, so drive it with a UI-independent check: either
    // outcome is a clean end, and an abort must not hang.
    let end = tokio::time::timeout(Duration::from_secs(10), run_turn(&mut r.repl.engine, &r.repl.console, "do it", &mut int)).await;
    assert!(matches!(end, Ok(TurnEnd::Done(..)) | Ok(TurnEnd::Aborted)), "must end promptly");
}

#[tokio::test]
async fn one_shot_prints_only_the_answer_on_stdout_and_exits_zero() {
    let mut r = rig("oneshot", ANSWER);
    r.repl = Repl::new(
        r.repl.engine,
        Console::new(
            Box::new(r.out.clone()),
            Box::new(r.err.clone()),
            ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
        ),
        None,
        false,
    );
    // run_turn needs the engine's Jev call to fail fast and route without judgment.
    let mut int = Presses { pending: vec![] };
    let code = r.repl.one_shot("change a", &mut int).await;
    assert_eq!(code, 0);
    assert_eq!(r.out.text(), "the answer\n");
    let e = r.err.text();
    assert!(e.contains("⏺ Edit a.txt") && e.contains("✓ claude"), "{e}");
    assert!(!e.contains("the answer"), "the answer is not repeated on stderr: {e}");
}

#[tokio::test]
async fn json_output_is_one_object_on_stdout_and_progress_stays_on_stderr() {
    let mut r = rig("oneshot-json", ANSWER);
    r.repl = Repl::new(
        r.repl.engine,
        Console::new(
            Box::new(r.out.clone()),
            Box::new(r.err.clone()),
            ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
        ),
        None,
        false,
    );
    let mut int = Presses { pending: vec![] };
    let code = r.repl.one_shot_as("change a", crate::tui::app::headless_output::OutputFormat::Json, &mut int).await;
    assert_eq!(code, 0);
    let out = r.out.text();
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.starts_with("{\"status\":\"answered\",\"exit_code\":0,\"answer\":\"the answer\"") && out.contains("\"files\":["), "{out}");
    assert!(r.err.text().contains("⏺ Edit a.txt"), "{}", r.err.text());
}

#[tokio::test]
async fn every_runs_the_prompt_again_until_the_times_are_done() {
    use crate::turn::domain::schedule::{Repeat, Schedule};
    let mut r = rig("repeat", ANSWER);
    r.repl = Repl::new(
        r.repl.engine,
        Console::new(
            Box::new(r.out.clone()),
            Box::new(r.err.clone()),
            ConsoleConfig { out_style: Style::off(), err_style: Style::off(), width: 80, live: false, project: r.dir.clone(), stream_answers: false },
        ),
        None,
        false,
    );
    let waits = Arc::new(Mutex::new(Vec::new()));
    let seen = waits.clone();
    let mut pause: crate::tui::repl::headless::Pause = Box::new(move |d| {
        seen.lock().unwrap().push(d);
        Box::pin(async {})
    });
    let repeat = Repeat { schedule: Schedule::Every(std::time::Duration::from_secs(600)), times: Some(3) };
    let mut int = Presses { pending: vec![] };
    let code = r.repl.repeat_shot("change a", crate::tui::app::headless_output::OutputFormat::Json, &repeat, &mut int, &mut pause).await;
    assert_eq!(code, 0);
    assert_eq!(r.out.text().lines().count(), 3, "one JSON line per run: {}", r.out.text());
    assert_eq!(*waits.lock().unwrap(), vec![std::time::Duration::from_secs(600); 2], "it waits between runs, not after the last");
    assert!(r.err.text().contains("run 2 answered; next in 600s"), "{}", r.err.text());
}

#[test]
fn only_a_long_turn_rings_the_bell_unless_it_is_off() {
    assert!(bell_after(Duration::from_secs(20), None));
    assert!(!bell_after(Duration::from_secs(3), None));
    assert!(!bell_after(Duration::from_secs(20), Some("off")));
    assert!(bell_after(Duration::from_secs(20), Some("on")));
}

#[test]
fn page_without_a_usable_pager_says_so() {
    // A program that does not exist cannot page; the caller then prints the text itself.
    std::env::set_var("PAGER", "/nonexistent/pager-for-test");
    assert!(!page("text"));
    std::env::remove_var("PAGER");
}

