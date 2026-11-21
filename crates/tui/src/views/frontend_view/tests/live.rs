use super::*;

#[test]
fn open_workers_are_listed_above_the_box_with_who_and_how_long() {
    use crate::app::screen_state::WorkerChip;
    let mut a = app();
    let chip = |id: &str, running: bool, age_ms: u64| WorkerChip { id: id.into(), provider: "gemini".into(), model: "argon".into(), running, age_ms, tools: 0, activity: None };
    a.workers = vec![chip("w1", true, 42_000), chip("w2", false, 90_000)];
    let text = screen(&a, 80);
    assert!(text.contains("▾ Workers · 1 running · 1 to review"), "{text}");
    assert!(text.contains("▶ w1  gemini · argon   running 0:42"), "{text}");
    assert!(text.contains("✓ w2  gemini · argon   ready to review"), "{text}");
    a.workers[0].tools = 7;
    a.workers[0].activity = Some("Bash cargo test".into());
    let text = screen(&a, 80);
    assert!(text.contains("running 0:42 · 7 tool calls"), "{text}");
    assert!(text.contains("⎿ Bash cargo test"), "{text}");
    a.workers = (0..6).map(|i| chip(&format!("w{i}"), true, 1000)).collect();
    assert!(screen(&a, 80).contains("… and 2 more"));
    assert!(!screen(&app(), 80).contains("Workers"));
}

#[test]
fn a_running_turn_shows_the_working_row_the_queue_and_the_keys_for_now() {
    let mut a = app();
    a.on_front(FrontMsg::TurnStarted);
    a.on_front(FrontMsg::Label("claude · sonnet-5-5".into()));
    a.on_front(FrontMsg::Activity("Bash cargo test".into()));
    a.queue.push_back("then run clippy".into());
    let s = screen(&a, 80);
    let work = s.lines().nth(1).unwrap_or_default();
    assert!(work.starts_with("  • Running cargo test 0s") && work.ends_with("claude · sonnet-5-5 · esc to stop"), "a still mark without colour: {work}");
    assert!(s.contains("\n  #1 then run clippy\n"), "{s}");
    assert!(s.contains("Type to queue a message for after this turn"), "{s}");
    assert_eq!(s.lines().last(), Some("  Enter:queue │ Shift+Enter:newline │ Shift+Tab:mode"), "Esc is said once, on the working row: {s}");
    assert_eq!(s.matches("esc").count(), 1, "{s}");
    let on = render(&a, 80, Instant::now(), ON).rows[1].clone();
    assert!(on.contains("\x1b[2mRunning") && on.contains("cargo test"), "the verb dim, what it acts on in the accent: {on:?}");
}

#[test]
fn the_working_row_says_what_is_happening_as_an_action() {
    assert_eq!(doing(None), "Thinking");
    assert_eq!(doing(Some("Bash cargo test")), "Running cargo test");
    assert_eq!(doing(Some("Read src/a.rs")), "Reading src/a.rs");
    assert_eq!(doing(Some("Grep TODO")), "Searching TODO");
    assert_eq!(doing(Some("stopping...")), "stopping...");
}

#[test]
fn an_approval_replaces_the_box_with_a_raised_card_of_numbered_choices() {
    let mut a = app();
    let (reply, _rx) = tokio::sync::oneshot::channel();
    a.on_front(FrontMsg::Approval(Box::new(ApprovalAsk {
        tool: "Bash".into(),
        notes: vec!["runs in the background".into()],
        lines: vec!["cargo test --quiet".into()],
        always: Some("Bash(cargo test:*)".into()),
        note: None,
        reply,
    })));
    let f = render(&a, 70, Instant::now(), OFF);
    let s = f.rows.join("\n");
    assert!(f.rows[1].starts_with("▌ Allow Bash?"), "{s}");
    assert!(s.contains("▌ cargo test --quiet"), "{s}");
    assert!(s.contains("▌ ● 1. Yes "), "{s}");
    assert!(s.contains("▌ ○ 2. Yes, and don't ask again (Bash(cargo test:*))"), "{s}");
    assert!(s.contains("▌ ○ 3. No, and tell the model what to do instead"), "{s}");
    assert!(f.rows.last().is_some_and(|r| r.starts_with("  1–3:choose │ ↑↓:move │ Enter:confirm │ Esc:no")), "{s}");
    assert!(!s.contains('╭'), "no box while the card is up: {s}");
    assert_eq!(f.cursor, None, "no text cursor on a card");
    let on = render(&a, 70, Instant::now(), ON);
    assert!(on.rows[1..7].iter().all(|r| r.contains("\x1b[48;5;236")), "the card is one raised surface: {:?}", on.rows);
}

#[test]
fn every_row_fits_one_column_short_of_the_terminal_at_every_width_and_style() {
    let mut a = app();
    typed(&mut a, "/");
    a.on_front(FrontMsg::TurnStarted);
    a.queue.push_back("日本語のメッセージ ".repeat(10));
    a.git = Some("main ±3".into());
    a.context = Some(91);
    for width in [1, 10, 20, 21, 40, 79, 80, 200] {
        for style in [ON, OFF] {
            let f = render(&a, width, Instant::now(), style);
            for row in &f.rows {
                assert!(display_width(row) < width.max(21), "{:?} at {width}", strip_ansi(row));
                assert!(!row.contains('\n'));
            }
            if !style.enabled {
                assert!(f.rows.iter().all(|r| !r.contains('\x1b')));
            }
        }
    }
}
