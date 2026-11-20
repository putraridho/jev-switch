use super::*;

#[test]
fn the_status_line_follows_the_live_terminal_width_not_the_width_at_startup() {
    let (mut c, err, screen) = on_screen(120);
    c.begin_turn();
    c.route_facts(&facts("claude", "claude-opus-5-5"));
    c.event(&Event::Tool(LONG_COMMAND.into()));
    for cols in [120, 70, 33, 61, 200, 5, 1] {
        screen.cols.store(cols, Ordering::SeqCst);
        err.clear();
        c.tick_at(Duration::from_secs(9));
        let drawn = frames(&err.text());
        assert_eq!(drawn.len(), 1, "one frame per tick at {cols} columns");
        let width = uifmt::display_width(&drawn[0]);
        assert!(width < cols.max(2), "a {width}-column frame reaches the last of {cols} columns: {:?}", drawn[0]);
        assert!(!drawn[0].contains('\n'), "one row: {:?}", drawn[0]);
    }
    assert_eq!(c.width(), 1);
}

#[test]
fn a_status_line_never_carries_control_characters_from_a_tool_summary() {
    let (mut c, err, _screen) = on_screen(80);
    c.begin_turn();
    for hostile in ["Bash echo \x1b[31mred\x1b[0m\r\nnext\x07 bell \x1b]0;title\x07 end \x1b", "Bash tab\there\x08\x7f\u{9b}31m", "\x1b"] {
        err.clear();
        c.event(&Event::Tool(hostile.into()));
        let all = err.text();
        // Erase, the tool line (only its final newline is a control character), erase, the status frame: two parts after each erase.
        let parts = frames(&all);
        assert_eq!(parts.len(), 2, "{all:?}");
        assert!(!parts[0].trim_end_matches('\n').chars().any(char::is_control), "{hostile:?} leaked into the tool line {:?}", parts[0]);
        assert!(!parts[1].chars().any(char::is_control), "{hostile:?} leaked into the status line {:?}", parts[1]);
    }
}

#[test]
fn keys_typed_during_a_turn_are_not_echoed_over_the_status_line() {
    let (c, _, screen) = on_screen(80);
    c.begin_turn();
    c.end_turn();
    assert_eq!(*screen.echo.lock().unwrap(), [false, true]);

    // A console that draws nothing does not touch the terminal's modes either.
    let (plain, _, _) = console(false, Style::off(), true);
    let quiet = FakeScreen::new(80);
    let plain = plain.with_screen(Box::new(quiet.clone()));
    plain.begin_turn();
    plain.end_turn();
    assert!(quiet.echo.lock().unwrap().is_empty());
}

#[test]
fn the_end_of_a_turn_restores_the_echo_the_terminal_had_not_always_on() {
    // Echo was already off (a `stty -echo` shell, a password-style wrapper): it stays off,
    // and the turn never touches it.
    let (c, _, err) = console(true, Style::off(), true);
    let off = FakeScreen::new(80).starting_with_echo(Some(false));
    let c = c.with_screen(Box::new(off.clone()));
    c.begin_turn();
    c.end_turn();
    assert!(off.echo.lock().unwrap().is_empty(), "no switch at all: {:?}", off.echo.lock().unwrap());
    assert_eq!(off.echo(), Some(false));
    drop(err);

    // Not a terminal: nothing to switch, nothing to restore.
    let (c, _, _) = console(true, Style::off(), true);
    let none = FakeScreen::new(80).starting_with_echo(None);
    let c = c.with_screen(Box::new(none.clone()));
    c.begin_turn();
    c.end_turn();
    assert!(none.echo.lock().unwrap().is_empty());
    assert_eq!(none.echo(), None);
}

#[test]
fn a_notice_waits_for_the_prompt_unless_a_turn_is_running() {
    let (c, _, err) = console(true, Style::off(), true);
    c.notify("codex is ready now");
    assert_eq!(err.text(), "", "at the prompt it must not land inside the line being typed");
    c.flush_notices();
    assert_eq!(err.text(), "  codex is ready now\n");
    c.flush_notices();
    assert_eq!(err.text(), "  codex is ready now\n", "printed once");

    err.clear();
    c.begin_turn();
    err.clear();
    c.notify("codex is ready now");
    let e = err.text();
    let at = e.find("  codex is ready now\n").expect(&e);
    assert!(e[..at].ends_with(uifmt::clear_line()), "the status line is erased first: {e:?}");
    assert!(e[at..].contains("routing"), "and drawn again under it: {e:?}");
}

#[test]
fn output_from_several_threads_never_splits_a_line() {
    let (c, _, err) = console(false, Style::off(), true);
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let c = c.clone();
            std::thread::spawn(move || {
                for j in 0..50 {
                    c.err(&format!("line-{i}-{j}-end"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let e = err.text();
    assert_eq!(e.lines().count(), 400);
    assert!(e.lines().all(|l| l.starts_with("line-") && l.ends_with("-end")), "{e}");
}
