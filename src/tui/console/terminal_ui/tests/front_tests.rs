use super::*;

/// Everything the front was asked to print, without colour.
fn printed(heard: &mut tokio::sync::mpsc::UnboundedReceiver<FrontMsg>) -> String {
    let mut out = String::new();
    while let Ok(msg) = heard.try_recv() {
        if let FrontMsg::Print(text) = msg {
            out.push_str(&uifmt::strip_ansi(&text));
            out.push('\n');
        }
    }
    out
}

#[test]
fn a_run_of_calls_prints_as_one_row_and_a_failed_call_keeps_its_own() {
    let (c, _, _) = console(false, Style::off(), true);
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    for i in 0..9 {
        c.event(&Event::Tool(format!("Bash cmd{i}")));
        c.event(&Event::ToolResult { ok: i != 7, text: format!("out{i}") });
    }
    c.event(&Event::Text("done".into()));
    c.end_turn();
    let e = printed(&mut heard);
    assert!(e.contains("  ◇ Ran 9 commands · 1 failed\n    ✗ Bash cmd7\n      out7\n"), "{e}");
    assert!(!e.contains("cmd0") && !e.contains("cmd8"), "calls that went well are counted, not listed: {e}");
}

#[test]
fn edits_join_the_run_with_their_lines_and_a_worker_call_stands_alone() {
    let (c, _, _) = console(false, Style::off(), true);
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    for i in 0..3 {
        c.event(&Event::Tool(format!("Read f{i}")));
        c.event(&Event::ToolResult { ok: true, text: "ok".into() });
    }
    c.event(&Event::Tool("Edit src/a.rs".into()));
    c.event(&Event::Edit { path: "/p/src/a.rs".into(), removed: vec!["x".into()], added: vec!["y".into(), "z".into()] });
    c.event(&Event::ToolResult { ok: true, text: "updated".into() });
    c.event(&Event::Tool("mcp__jev__delegate write the tests".into()));
    c.event(&Event::Text("done".into()));
    c.end_turn();
    let e = printed(&mut heard);
    assert!(e.contains("  ◇ Edited a.rs, explored 3 files +2 −1\n  ⇉ Delegate write the tests\n"), "the run ends where the worker call starts: {e}");
    assert!(!e.contains("Read f0"), "{e}");
}

#[test]
fn the_answer_is_previewed_while_it_is_written_and_committed_once() {
    let (c, _, _) = console(false, Style::off(), true);
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    c.begin_turn();
    c.event(&Event::TextDelta("Hel".into()));
    c.event(&Event::TextDelta("lo".into()));
    let mut deltas = 0;
    while let Ok(msg) = heard.try_recv() {
        match msg {
            FrontMsg::AnswerDelta(t) => {
                deltas += 1;
                assert!(t == "Hel" || t == "lo", "{t}");
            }
            FrontMsg::AnswerReset => panic!("nothing finished yet"),
            _ => {}
        }
    }
    assert_eq!(deltas, 2);
    c.event(&Event::Text("Hello".into()));
    c.end_turn();
    let mut seen = Vec::new();
    while let Ok(msg) = heard.try_recv() {
        match msg {
            FrontMsg::AnswerReset => seen.push("reset"),
            FrontMsg::Print(t) => {
                assert!(uifmt::strip_ansi(&t).contains("Hello"), "{t}");
                seen.push("print");
            }
            _ => {}
        }
    }
    assert_eq!(seen, vec!["reset", "print"], "the preview goes, then the finished answer once");
}

#[test]
fn a_tool_after_streamed_text_drops_the_preview_because_it_was_narration() {
    let (c, _, _) = console(false, Style::off(), true);
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    c.begin_turn();
    c.event(&Event::TextDelta("I'll look".into()));
    c.event(&Event::Tool("Read a.txt".into()));
    let mut reset = false;
    while let Ok(msg) = heard.try_recv() {
        match msg {
            FrontMsg::AnswerReset => reset = true,
            FrontMsg::Print(t) => assert!(!uifmt::strip_ansi(&t).contains("I'll look"), "{t}"),
            _ => {}
        }
    }
    assert!(reset, "the preview is dropped when the text turns out to be narration");
}

#[test]
fn reasoning_stays_off_the_scrollback_and_shows_as_the_latest_line_on_the_working_row() {
    let (c, _, err) = console(false, Style::off(), true);
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    c.event(&Event::Thinking("First I will look at run.rs.\n\nThen the screen code.".into()));
    assert!(!err.text().contains("run.rs") && !err.text().contains("screen code"), "{}", err.text());
    let mut activity = None;
    while let Ok(msg) = heard.try_recv() {
        if let FrontMsg::Activity(t) = msg {
            activity = Some(t);
        }
    }
    assert_eq!(activity.as_deref(), Some("Thinking · Then the screen code."));
}

#[test]
fn a_front_gets_messages_for_the_turn_and_hears_when_the_model_took_one() {
    let (mut c, _, _) = console(false, Style::off(), true);
    assert!(c.steering().is_none(), "no input during a turn: nothing to steer");
    assert!(!c.steer("hi"));
    let (front, mut heard) = Front::channel();
    let mut c = c.with_front(front);
    let mut turn = c.steering().expect("a front types during the turn");
    assert!(c.steer("also run clippy"));
    assert_eq!(turn.try_recv().unwrap(), "also run clippy");
    c.event(&Event::Steered("also run clippy".into()));
    let mut steered = None;
    while let Ok(msg) = heard.try_recv() {
        if let FrontMsg::Steered(t) = msg {
            steered = Some(t);
        }
    }
    assert_eq!(steered.as_deref(), Some("also run clippy"));
    // The turn is over: its receiver is gone, and a message can no longer be offered.
    drop(turn);
    assert!(!c.steer("too late"));
}

#[test]
fn a_message_taken_back_out_of_the_queue_is_not_also_sent_to_the_turn() {
    let (front, _heard) = Front::channel();
    let (c, _, _) = console(false, Style::off(), true);
    let mut c = c.with_front(front);
    let _turn = c.steering().unwrap();
    let wanted = c.steer_wanted().expect("a console can withdraw");
    // Offered, then taken back to be edited: the turn skips that copy once...
    assert!(c.steer("run the tests"));
    c.withdraw_steer("run the tests");
    assert!(!wanted("run the tests"), "the withdrawn copy is skipped");
    // ...and the edited message sent afterwards goes through.
    assert!(wanted("run the tests and clippy"));
    // Sending the same text again after taking it back is the one copy that counts.
    assert!(wanted("run the tests"), "a withdrawal skips exactly one copy");
}

#[test]
fn commands_typed_during_a_turn_are_never_written_into_it() {
    let (front, _heard) = Front::channel();
    let (mut c, _, _) = console(false, Style::off(), true);
    c = c.with_front(front);
    let mut turn = c.steering().unwrap();
    for command in ["/clear", "/model sonnet", "!git status", "  /new", "/review-pr 12", "# remember this"] {
        assert!(!c.steer(command), "{command}");
    }
    assert!(turn.try_recv().is_err());
    assert!(c.steer("also run clippy") && turn.try_recv().is_ok());
}

#[test]
fn the_line_editor_queue_drops_a_message_the_model_took() {
    let (c, _, err) = console(false, Style::off(), true);
    c.queue_message("first".into(), false);
    c.queue_message("second".into(), false);
    let mut c2 = c.clone();
    c2.event(&Event::Steered("first".into()));
    assert_eq!(c.queued_len(), 1);
    assert_eq!(c.take_queued().as_deref(), Some("second"));
    assert!(err.text().contains("(taken into the running turn)"), "{}", err.text());
}
