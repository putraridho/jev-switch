//! `/project`, project memory and auto-compaction, through the REPL with the fake model.

use super::*;
use crate::ctxintel::AutoCompact;
use crate::projectkind::Check;

/// A rig whose engine saves under a state folder of its own, so notes have somewhere to live.
fn rig_with_state(name: &str, script: &str) -> Rig {
    let mut r = rig(name, script);
    let state = r.dir.join(".state");
    std::fs::create_dir_all(&state).unwrap();
    r.repl.engine.set_state_dir(Some(state));
    r
}

#[tokio::test]
async fn notes_are_kept_listed_forgotten_and_start_the_next_session() {
    let mut r = rig_with_state("proj-notes", "read -r first; printf 'FIN:%s\\n' \"$first\"");
    run(&mut r, "/project notes").await;
    assert!(r.out.text().contains("No notes"), "{}", r.out.text());
    run(&mut r, "/project note tests need docker up").await;
    run(&mut r, "/project note never edit vendor/").await;
    r.out.clear();
    run(&mut r, "/project notes").await;
    let o = r.out.text();
    assert!(o.contains(" 1. tests need docker up") && o.contains(" 2. never edit vendor/"), "{o}");
    // The first message of a session carries them; a later one does not.
    assert!(r.repl.with_carry("hello").starts_with("Notes the user keeps for this project (project memory):\n- tests need docker up"));
    send(&mut r, "hello").await;
    assert_eq!(r.repl.with_carry("again"), "again");
    // A new session starts with them again, and they show in /context.
    run(&mut r, "/new").await;
    assert!(r.repl.with_carry("x").contains("never edit vendor/"));
    r.out.clear();
    run(&mut r, "/context").await;
    assert!(r.out.text().contains("memory"), "{}", r.out.text());
    run(&mut r, "/project forget 1").await;
    assert!(!r.repl.with_carry("x").contains("docker"));
    r.out.clear();
    run(&mut r, "/project forget 9").await;
    assert!(r.out.text().contains("there is no note 9"), "{}", r.out.text());
}

#[tokio::test]
async fn the_map_lists_source_files_and_goes_with_the_next_message() {
    let mut r = rig("proj-map", ANSWER);
    std::fs::write(r.dir.join("lib.rs"), "pub fn alpha() {}\npub struct Beta;\n").unwrap();
    run(&mut r, "/project map").await;
    let o = r.out.text();
    assert!(o.contains("lib.rs") && o.contains("fn alpha") && o.contains("struct Beta") && o.contains("goes with your next message"), "{o}");
    assert!(r.repl.with_carry("go").contains("Map of this repository"));
    r.out.clear();
    run(&mut r, "/project map nothing-matches").await;
    assert!(!r.out.text().contains("lib.rs"), "{}", r.out.text());
}

#[tokio::test]
async fn the_overview_names_the_kind_and_the_commands_and_a_plain_folder_has_none() {
    let mut r = rig("proj-overview", ANSWER);
    run(&mut r, "/project").await;
    assert!(r.out.text().contains("No Cargo.toml"), "{}", r.out.text());
    std::fs::write(r.dir.join("Cargo.toml"), "").unwrap();
    r.out.clear();
    run(&mut r, "/project").await;
    assert!(r.out.text().contains("Rust (cargo)") && r.out.text().contains("cargo test"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/project check bogus").await;
    assert!(r.out.text().contains("not a check"), "{}", r.out.text());
}

#[tokio::test]
async fn a_failing_check_is_noted_and_without_fix_no_model_is_asked() {
    let mut r = rig("proj-check", "printf 'FIN:never asked\\n'");
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.run_check_command(Check::Test, "echo boom; exit 3", false, &mut io).await;
    let o = r.out.text();
    assert!(o.contains("boom") && o.contains("test failed"), "{o}");
    assert!(r.repl.carry[0].contains("it exited 3"));
    assert!(r.repl.log().is_empty(), "no turn ran");
}

#[tokio::test]
async fn the_fix_loop_asks_the_model_then_reruns_until_the_check_passes() {
    // The check passes once the file exists; the fake model creates it when asked.
    let mut r = rig("proj-fix", "printf 'FIN:fixed it\\n'; touch fixed");
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.run_check_command(Check::Test, "test -f fixed", true, &mut io).await;
    let o = r.out.text();
    assert!(o.contains("✓ test passed"), "{o}");
    assert_eq!(r.repl.log().turns().len(), 1, "one fix round was enough");
    assert!(r.repl.log().turns()[0].prompt.contains("test check (`test -f fixed`) failed"));
    assert!(r.repl.carry.is_empty(), "the failure output went with the fix request");
}

#[tokio::test]
async fn the_fix_loop_gives_up_after_its_rounds() {
    let mut r = rig("proj-fix-giveup", "printf 'FIN:tried\\n'");
    let mut int = Presses { pending: vec![] };
    let mut io = Io { interrupt: &mut int, confirm: None };
    r.repl.run_check_command(Check::Lint, "exit 1", true, &mut io).await;
    assert!(r.out.text().contains("still fails after 3 fix attempts"), "{}", r.out.text());
    assert_eq!(r.repl.log().turns().len(), project_cmds::FIX_ROUNDS - 1);
}

#[tokio::test]
async fn a_full_window_compacts_before_the_next_message_unless_switched_off() {
    let mut r = rig("proj-autocompact", "printf 'FIN:the summary\\n'");
    assert_eq!(r.repl.auto_compact(), AutoCompact::At(90), "on by default");
    send(&mut r, "one").await;
    send(&mut r, "two").await;
    assert!(!r.repl.auto_compact().due(r.repl.context_percent(), 2), "2000 tokens of a large window: not due");
    // Two earlier turns that filled 95% of a Claude window.
    let spec = crate::catalog::models().iter().find(|m| m.provider == ProviderId::Claude).unwrap();
    let mut full = r.repl.log().turns()[1].clone();
    full.route = Some((ProviderId::Claude, spec.id.to_string()));
    full.usage = Some(Usage { input_tokens: spec.context_tokens * 95 / 100, cached_input_tokens: 0, output_tokens: 10, context_tokens: Some(spec.context_tokens * 95 / 100) });
    r.repl.log.push(full);
    assert_eq!(r.repl.context_percent(), Some(95));
    r.out.clear();
    send(&mut r, "three").await;
    let o = r.out.text();
    assert!(o.contains("context is 95% full: compacting before this message") && o.contains("compacted 3 turns"), "{o}");
    assert_eq!(r.repl.log().turns().len(), 1, "a fresh session holds only the message that followed the summary");
    assert_eq!(r.repl.log().turns()[0].prompt, "three");
}
