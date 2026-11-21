//! Commands on the session and its settings.

use super::*;

#[tokio::test]
async fn settings_commands_change_the_engine_and_say_so() {
    let mut r = rig("settings", ANSWER);
    run(&mut r, "/model codex").await;
    assert_eq!(r.repl.engine.pin().to_string(), "codex");
    run(&mut r, "/provider off").await;
    assert!(!r.repl.engine.enabled());
    run(&mut r, "/orchestrate on").await;
    assert!(r.repl.engine.orchestrate());
    run(&mut r, "/thinking on").await;
    assert!(r.repl.console.show_thinking());
    run(&mut r, "/thinking").await;
    run(&mut r, "/thinking off").await;
    assert!(!r.repl.console.show_thinking());
    run(&mut r, "/model").await;
    run(&mut r, "/model nonsense").await;
    let o = r.out.text();
    assert!(o.contains("model: codex") && o.contains("routing: off (Claude only)") && o.contains("orchestrate: on"), "{o}");
    assert!(o.contains("thinking: on") && o.contains("thinking: off"), "{o}");
    assert!(o.contains("⚠ unknown provider"), "{o}");
    run(&mut r, "/providers").await;
    assert!(r.out.text().contains("● claude"), "{}", r.out.text());
}

#[tokio::test]
async fn new_and_resume_without_a_saved_session_say_so_plainly() {
    let mut r = rig("resume", ANSWER);
    run(&mut r, "/resume").await;
    assert!(r.out.text().contains("Nothing to resume"), "{}", r.out.text());
    run(&mut r, "/new").await;
    assert!(r.out.text().contains("✓ new session"));
}

#[tokio::test]
async fn cache_and_effort_commands_read_and_set_the_session() {
    let mut r = rig("cache-effort", ANSWER);
    run(&mut r, "/cache").await;
    assert!(r.out.text().contains("No cache data yet"), "{}", r.out.text());
    run(&mut r, "/effort").await;
    assert!(r.out.text().contains("effort: default"), "{}", r.out.text());
    run(&mut r, "/effort high").await;
    assert!(r.out.text().contains("effort: high") && r.repl.engine.effort().to_string() == "high", "{}", r.out.text());
    send(&mut r, "change a").await;
    r.out.clear();
    run(&mut r, "/cache").await;
    let o = r.out.text();
    assert!(o.contains("Cache by provider") && o.contains("claude") && o.contains("hit  50%") && o.contains("Warm now"), "{o}");
    r.out.clear();
    run(&mut r, "/cost").await;
    assert!(r.out.text().contains("Cache by provider"), "{}", r.out.text());
}

#[tokio::test]
async fn cost_context_copy_and_export_read_the_session() {
    let mut r = rig("session-cmds", ANSWER);
    run(&mut r, "/cost").await;
    run(&mut r, "/context").await;
    run(&mut r, "/export").await;
    let o = r.out.text();
    assert!(o.contains("No turns in this session yet") && o.contains("No turn has run yet") && o.contains("Nothing to export yet"), "{o}");
    send(&mut r, "change a").await;
    r.out.clear();
    run(&mut r, "/cost").await;
    let o = r.out.text();
    assert!(o.contains("This session") && o.contains("1 turn") && o.contains("in     2K (cached     1K)"), "{o}");
    r.out.clear();
    run(&mut r, "/context").await;
    let o = r.out.text();
    assert!(o.contains("2K of") && o.contains("turns       1"), "{o}");
    r.out.clear();
    run(&mut r, "/export session.md").await;
    assert!(r.out.text().contains("✓ saved 1 turns to"), "{}", r.out.text());
    let md = std::fs::read_to_string(r.dir.join("session.md")).unwrap();
    assert!(md.contains("> change a") && md.contains("the answer"), "{md}");
    r.out.clear();
    run(&mut r, "/export session.md").await;
    assert!(r.out.text().contains("exists already"), "an export never overwrites: {}", r.out.text());
    assert_eq!(r.repl.log().last_answer(), Some("the answer"));
    run(&mut r, "/new").await;
    assert!(r.repl.log().is_empty(), "a new session starts a new log");
}

#[tokio::test]
async fn a_shell_command_shows_its_output_and_goes_with_the_next_message() {
    // The fake model echoes what it was sent, so the note can be seen arriving.
    let mut r = rig("bang", "read -r first; printf 'FIN:%s\\n' \"$first\"");
    run(&mut r, "!echo hello-from-shell").await;
    let o = r.out.text();
    assert!(o.contains("hello-from-shell") && o.contains("✓ exit 0") && o.contains("goes with your next message"), "{o}");
    assert_eq!(r.repl.carry.len(), 1);
    assert!(r.repl.with_carry("now fix it").starts_with("I ran `echo hello-from-shell` in the shell; it exited 0."));
    assert!(r.repl.with_carry("now fix it").ends_with("---\n\nnow fix it"));
    send(&mut r, "now fix it").await;
    assert!(r.repl.carry.is_empty(), "the note went with the message");
    assert_eq!(r.repl.log().turns()[0].prompt, "now fix it", "the log keeps what the person typed");
}

#[tokio::test]
async fn a_failed_turn_keeps_the_notes_for_the_next_message() {
    let mut r = rig("bang-fail", "printf 'FAIL:segmentation fault\\n'");
    run(&mut r, "!echo kept").await;
    send(&mut r, "use it").await;
    assert_eq!(r.repl.carry.len(), 1, "the shell output still waits for a turn that answers");
}

#[tokio::test]
async fn why_config_and_terminal_setup_print_their_screens() {
    let mut r = rig("why", ANSWER);
    run(&mut r, "/why").await;
    assert!(r.out.text().contains("No turn has been routed yet"), "{}", r.out.text());
    send(&mut r, "change a").await;
    r.out.clear();
    run(&mut r, "/why").await;
    let o = r.out.text();
    assert!(o.contains("Last route") && o.contains("claude") && o.contains("setting"), "{o}");
    r.out.clear();
    run(&mut r, "/config").await;
    let o = r.out.text();
    assert!(o.contains("mode") && o.contains("JEV_API_KEY") && o.contains("permissions"), "{o}");
    assert!(!o.contains("sk-"), "no key value is ever shown");
    r.out.clear();
    run(&mut r, "/terminal-setup").await;
    assert!(r.out.text().contains("Shift-Enter for a new line"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/mcp").await;
    assert!(r.out.text().contains("MCP servers"), "{}", r.out.text());
}

#[tokio::test]
async fn compact_starts_fresh_with_the_summary_waiting() {
    let mut r = rig("compact", "printf 'FIN:SUMMARY of it all\\n'");
    run(&mut r, "/compact").await;
    assert!(r.out.text().contains("Nothing to compact yet"), "{}", r.out.text());
    send(&mut r, "first").await;
    let mode = r.repl.engine.mode();
    r.out.clear();
    run(&mut r, "/compact keep names").await;
    let o = r.out.text();
    assert!(o.contains("✓ compacted 1 turn"), "{o}");
    assert_eq!(r.repl.engine.mode(), mode, "the mode is put back after the read-only summary");
    assert!(r.repl.log().is_empty());
    assert!(r.repl.with_carry("next").starts_with("Summary of our conversation so far") && r.repl.with_carry("next").contains("SUMMARY of it all"));
}

#[tokio::test]
async fn review_runs_read_only_and_puts_the_mode_back() {
    let mut r = rig("review", ANSWER);
    run(&mut r, "/mode auto").await;
    send(&mut r, "/review").await;
    assert!(r.err.text().contains("/review runs in plan mode"), "{}", r.err.text());
    assert_eq!(r.repl.engine.mode(), jev_approval::domain::mode::Mode::Auto);
    assert_eq!(r.repl.log().turns()[0].prompt, "/review");
}

#[tokio::test]
async fn permissions_lists_adds_removes_and_adds_folders() {
    let mut r = rig("perms", ANSWER);
    run(&mut r, "/permissions").await;
    let o = r.out.text();
    assert!(o.contains("Permissions") && o.contains("none yet"), "{o}");
    r.out.clear();

    run(&mut r, "/permissions allow Bash(cargo test:*)").await;
    run(&mut r, "/permissions allow Bash(cargo test:*)").await;
    let o = r.out.text();
    assert!(o.contains("✓ allowed Bash(cargo test:*)") && o.contains("already allowed"), "{o}");
    r.out.clear();

    std::fs::create_dir_all(r.dir.join("data")).unwrap();
    run(&mut r, "/permissions dir data").await;
    run(&mut r, "/permissions dir nowhere").await;
    let o = r.out.text();
    assert!(o.contains("✓ added folder") && o.contains("data") && o.contains("⚠"), "{o}");
    r.out.clear();

    run(&mut r, "/permissions").await;
    let o = r.out.text();
    assert!(o.contains("Bash(cargo test:*)") && o.contains("additional folders"), "{o}");
    let file = std::fs::read_to_string(r.dir.join(".claude/settings.local.json")).unwrap();
    assert!(file.contains("Bash(cargo test:*)") && file.contains("additionalDirectories"), "{file}");
    r.out.clear();

    run(&mut r, "/permissions remove Bash(cargo test:*)").await;
    run(&mut r, "/permissions remove Bash(cargo test:*)").await;
    let o = r.out.text();
    assert!(o.contains("✓ removed Bash(cargo test:*)") && o.contains("is not in the project's rules"), "{o}");
    r.out.clear();

    run(&mut r, "/permissions allow nonsense(").await;
    assert!(r.out.text().contains("not a permission rule"), "{}", r.out.text());
    std::fs::write(r.dir.join(".claude/settings.local.json"), "{ broken").unwrap();
    r.out.clear();
    run(&mut r, "/permissions allow WebSearch").await;
    assert!(r.out.text().contains("not valid JSON") && r.out.text().contains("left as it is"), "{}", r.out.text());
    assert_eq!(std::fs::read_to_string(r.dir.join(".claude/settings.local.json")).unwrap(), "{ broken");
}

