//! Type-ahead tests: a chunk through the console.

use super::*;

#[test]
fn a_chunk_through_the_console_queues_and_reports_esc() {
    use crate::testkit::capture::Buf;
    use crate::tui::console::terminal_ui::ConsoleConfig;
    let console = Console::new(
        Box::new(Buf::default()),
        Box::new(Buf::default()),
        ConsoleConfig { out_style: crate::tui::render::Style::off(), err_style: crate::tui::render::Style::off(), width: 80, live: false, project: "/tmp".into(), stream_answers: true },
    );
    let mut d = Draft::default();
    assert!(!handle_chunk(&console, &mut d, b"first\r"));
    assert!(!handle_chunk(&console, &mut d, b"urgent"));
    assert!(handle_chunk(&console, &mut d, b"\x1b"));
    assert_eq!(console.take_queued(), Some("urgent".to_string()), "a message sent with Esc goes first");
    assert_eq!(console.take_queued(), Some("first".to_string()));
    assert_eq!(console.take_queued(), None);
}
