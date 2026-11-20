use super::*;

#[test]
fn a_plain_console_prints_route_tools_and_answers_without_any_escape() {
    let (mut c, out, err) = console(false, Style::off(), true);
    c.route_facts(&facts("claude", "sonnet-5-5"));
    c.event(&Event::Tool("Edit /proj/src/a.rs".into()));
    c.event(&Event::Hint("✗ denied Bash".into()));
    c.event(&Event::Text("the answer".into()));
    let e = err.text();
    assert!(e.contains("  → claude · sonnet-5-5   explain · first turn"), "{e}");
    assert!(e.contains("  ⏺ Edit src/a.rs"), "{e}");
    assert!(e.contains("  ✗ denied Bash"), "{e}");
    assert_eq!(out.text(), "the answer\n");
    assert!(!e.contains('\x1b') && !out.text().contains('\x1b'));
}

#[test]
fn answers_do_not_stream_in_print_mode_but_hints_still_reach_stderr() {
    let (mut c, out, err) = console(false, Style::off(), false);
    c.event(&Event::Text("the answer".into()));
    c.event(&Event::Hint("✗ denied Bash".into()));
    assert_eq!(out.text(), "");
    assert!(err.text().contains("denied Bash"));
    // A hint is never answer text, whatever it starts with; answer text never goes to stderr.
    let (mut s, out, err) = console(false, Style::off(), true);
    s.event(&Event::Hint("denied Bash".into()));
    s.event(&Event::Text("✗ is a fine way to start an answer".into()));
    assert_eq!(out.text(), "✗ is a fine way to start an answer\n");
    assert!(err.text().contains("denied Bash") && !err.text().contains("fine way"), "{}", err.text());
}

#[test]
fn the_answer_is_laid_out_for_the_terminal_when_styled() {
    let (mut c, out, _) = console(false, Style::on(), true);
    c.event(&Event::Text("# Title\n\nuse `cargo test`".into()));
    let o = out.text();
    assert!(o.contains("\x1b[1mTitle") && o.contains("\x1b[36m"), "{o:?}");
}

#[test]
fn a_live_console_draws_the_status_line_and_keeps_lines_from_interleaving_with_it() {
    let (mut c, _, err) = console(true, Style::off(), true);
    c.begin_turn();
    let first = err.text();
    assert!(first.starts_with(uifmt::hide_cursor()), "{first:?}");
    assert!(first.contains("\r\x1b[2K") && first.contains("routing"), "{first:?}");
    err.clear();

    c.route_facts(&facts("codex", "gpt-6"));
    c.tick_at(Duration::from_millis(2500));
    c.event(&Event::Tool("Bash cargo test".into()));
    let e = err.text();
    // Every printed line is preceded by an erase, and followed by a redraw of the status.
    let erase = uifmt::clear_line();
    let route_at = e.find("→ codex · gpt-6").expect("route line");
    assert!(e[..route_at].ends_with(&format!("{erase}  ")), "the status line is erased before the route line: {e:?}");
    let tool_at = e.find("⏺ Bash cargo test").expect("tool line");
    let status_after_tool = &e[tool_at..];
    assert!(status_after_tool.contains("codex · gpt-6") && status_after_tool.contains("Bash cargo test"), "{e:?}");
    assert!(e.contains("2s"), "the tick shows the elapsed time: {e:?}");
    c.end_turn();
}

#[test]
fn ending_a_turn_erases_the_status_line_and_shows_the_cursor() {
    let (c, _, err) = console(true, Style::off(), true);
    c.begin_turn();
    err.clear();
    c.end_turn();
    assert_eq!(err.text(), format!("{}{}", uifmt::clear_line(), uifmt::show_cursor()));
    // After it, ticking draws nothing.
    err.clear();
    c.tick();
    assert_eq!(err.text(), "");
}

#[test]
fn a_console_that_is_not_live_never_draws_anything() {
    let (c, _, err) = console(false, Style::off(), true);
    c.begin_turn();
    c.tick();
    c.end_turn();
    assert_eq!(err.text(), "");
}

#[test]
fn a_failover_waits_for_the_next_route_and_names_it() {
    let (mut c, _, err) = console(false, Style::off(), true);
    c.route_facts(&facts("claude", "opus"));
    err.clear();
    let kind = quota();
    c.failover(&Failover {
        provider: ProviderId::Claude,
        model: "opus",
        kind: &kind,
        message: "limit",
        discarded: false,
        will_retry: true,
        text: "x",
    });
    assert_eq!(err.text(), "", "nothing yet: the next model is not known");
    c.route_facts(&facts("codex", "gpt-6"));
    let e = err.text();
    let note_at = e.find("⚠ claude hit its limit (resets 5pm) → trying codex · gpt-6").expect(&e);
    assert!(note_at < e.find("→ codex · gpt-6   ").unwrap(), "the note comes before the route line: {e}");
}

#[test]
fn a_failover_that_ends_the_turn_is_flushed_without_a_next_model() {
    let (mut c, _, err) = console(false, Style::off(), true);
    let kind = quota();
    c.failover(&Failover {
        provider: ProviderId::Claude,
        model: "opus",
        kind: &kind,
        message: "limit",
        discarded: true,
        will_retry: true,
        text: "x",
    });
    c.end_turn();
    let e = err.text();
    assert!(e.contains("⚠ claude hit its limit (resets 5pm) (partial output discarded)") && !e.contains("trying"), "{e}");
}

#[test]
fn a_final_failure_is_an_error_block_with_next_steps_and_no_debug_output() {
    let (mut c, _, err) = console(false, Style::off(), true);
    let kind = ErrorKind::Auth;
    c.failover(&Failover {
        provider: ProviderId::Codex,
        model: "gpt-6",
        kind: &kind,
        message: "401 Unauthorized",
        discarded: false,
        will_retry: false,
        text: "[error] codex gpt-6 failed: 401 Unauthorized",
    });
    let e = err.text();
    assert!(e.starts_with("  ✗ Login needed\n    codex · gpt-6: 401 Unauthorized\n    Try:\n      • Run `codex login`"), "indented like the turn's rows: {e}");
    assert!(!e.contains("Auth") && !e.contains("{"), "no Debug output: {e}");
}

#[test]
fn no_route_for_a_task_is_left_for_the_caller_and_a_bad_pin_is_explained() {
    let (mut c, _, err) = console(false, Style::off(), true);
    c.no_route(&NoRouteInfo::Nothing { task: "security" }, "text");
    assert_eq!(err.text(), "");
    assert_eq!(c.take_no_route().as_deref(), Some("security"));
    assert_eq!(c.take_no_route(), None, "taken once");

    c.no_route(&NoRouteInfo::Pin { what: "codex:gpt-6 is not usable now." }, "text");
    let e = err.text();
    assert!(e.contains("✗ Your /model pin can't run right now") && e.contains("codex:gpt-6 is not usable now\n"), "{e}");
    assert!(e.contains("`/model auto`") && e.contains("`/providers`"), "{e}");
}

#[test]
fn a_cancelled_turn_prints_nothing_of_its_own() {
    let (mut c, out, err) = console(false, Style::off(), true);
    c.cancelled("[cancelled] stopped by the user");
    assert_eq!((out.text(), err.text()), (String::new(), String::new()));
}

#[test]
fn plain_notes_become_warning_lines() {
    assert_eq!(note_line("[jev] unavailable (timeout); routing without its judgment", Style::off()), "  ⚠ jev: unavailable (timeout); routing without its judgment");
    assert_eq!(note_line("something else", Style::off()), "  ⚠ something else");
    let (mut c, _, err) = console(false, Style::off(), true);
    c.note("[orchestrate] could not write x\n\nsecond");
    assert_eq!(err.text(), "  ⚠ orchestrate: could not write x\n  ⚠ second\n");
}

#[test]
fn the_end_of_a_turn_shows_the_summary_files_and_how_to_review_them() {
    let (c, _, err) = console(false, Style::off(), true);
    let summary = SummaryFacts { ok: true, provider: "claude".into(), model: "sonnet".into(), elapsed_ms: 14_000, files_changed: Some((1, 3, 1)), ..SummaryFacts::default() };
    let files = [FileChange { status: 'M', path: "src/a.rs".into(), added: 3, removed: 1 }];
    c.turn_end(&summary, &files);
    let e = err.text();
    assert!(e.starts_with("  ✓ claude · sonnet · 14s"), "{e}");
    assert!(e.contains("  M src/a.rs") && e.contains("+3 −1"), "{e}");
    assert!(e.contains("  /diff to review, /undo to revert\n"), "{e}");
    assert!(e.ends_with("\n\n"), "a blank line separates turns: {e:?}");

    err.clear();
    c.turn_end(&SummaryFacts { ok: true, cancelled: true, elapsed_ms: 3000, ..SummaryFacts::default() }, &[]);
    let e = err.text();
    assert!(e.contains("■ stopped · 3s") && !e.contains("/diff"), "{e}");
}
