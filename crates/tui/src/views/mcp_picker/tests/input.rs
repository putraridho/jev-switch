use super::*;
use jev_backend::infrastructure::mcp_discovery::{McpRow, McpTransport};
use jev_backend::infrastructure::mcp_server::group;

#[test]
fn the_cursor_moves_pages_and_stops_at_the_ends() {
    let rows: Vec<McpRow> = (0..6)
        .map(|i| row(&format!("s{i}"), McpScope::Claude, McpTransport::Http))
        .collect();
    let mut p = McpPicker::new(group(&rows), 2);
    p.press(Key::Up);
    for _ in 0..3 {
        p.press(Key::Down);
    }
    let t = text(&p);
    assert!(
        t.contains("❯ ◌ s3") && !t.contains("s0") && t.contains("3-4 of 6"),
        "{t}"
    );
    p.press(Key::PageDown);
    p.press(Key::PageDown);
    assert!(text(&p).contains("❯ ◌ s5"));
    p.press(Key::PageUp);
    assert!(text(&p).contains("❯ ◌ s3"));
}

#[test]
fn typing_narrows_and_esc_clears_before_closing() {
    let mut p = McpPicker::new(servers(), 10);
    for c in "POST".chars() {
        p.press(Key::Char(c));
    }
    let t = text(&p);
    assert!(t.contains("❯ ◌ posthog") && !t.contains("luna"), "{t}");
    assert_eq!(p.press(Key::Cancel), Outcome::Continue);
    assert!(text(&p).contains("luna"), "Esc cleared the search");
    assert_eq!(p.press(Key::Cancel), Outcome::Cancelled);
    let mut p = McpPicker::new(servers(), 10);
    p.press(Key::Char('z'));
    assert!(text(&p).contains("No server matches."));
    assert_eq!(p.press(Key::Enter), Outcome::Continue, "nothing to open");
    p.press(Key::Backspace);
    assert!(text(&p).contains("luna"));
    assert_eq!(
        McpPicker::new(servers(), 10).press(Key::Interrupt),
        Outcome::Cancelled
    );
}

#[test]
fn providers_and_transport_are_searchable() {
    let mut p = McpPicker::new(servers(), 10);
    for c in "codex".chars() {
        p.press(Key::Char(c));
    }
    let t = text(&p);
    assert!(
        t.contains("luna") && !t.contains("posthog") && !t.contains(" pw"),
        "{t}"
    );
}

#[test]
fn enter_opens_the_card_and_its_logins() {
    let mut p = McpPicker::new(servers(), 10);
    p.press(Key::Enter);
    let t = text(&p);
    assert!(t.starts_with("luna"), "{t}");
    assert!(t.contains("https://luna.example/mcp"), "{t}");
    assert!(
        t.contains("❯ Log in with claude") && t.contains("Log in with codex") && t.contains("Back"),
        "{t}"
    );
    assert_eq!(
        p.press(Key::Enter),
        Outcome::Login {
            server: "luna".into(),
            scope: McpScope::Claude
        }
    );
    p.press(Key::Down);
    assert_eq!(
        p.press(Key::Enter),
        Outcome::Login {
            server: "luna".into(),
            scope: McpScope::Codex
        }
    );
}

#[test]
fn back_left_and_esc_return_to_the_list_where_it_was() {
    let mut p = McpPicker::new(servers(), 10);
    p.press(Key::Down);
    p.press(Key::Down);
    p.press(Key::Right);
    assert!(text(&p).starts_with("posthog"), "{}", text(&p));
    p.press(Key::Left);
    assert!(text(&p).contains("❯ ◌ posthog"), "{}", text(&p));
    for c in "ost".chars() {
        p.press(Key::Char(c));
    }
    p.press(Key::Enter);
    p.press(Key::Left);
    let t = text(&p);
    assert!(
        t.contains("❯ ◌ posthog") && t.contains("Search: ost"),
        "{t}"
    );
    p.press(Key::Enter);
    p.press(Key::Cancel);
    assert!(text(&p).contains("❯ ◌ posthog"));
    p.press(Key::Enter);
    p.press(Key::Down);
    assert_eq!(p.press(Key::Enter), Outcome::Continue, "Back");
    assert!(text(&p).contains("❯ ◌ posthog"));
    p.press(Key::Enter);
    assert_eq!(p.press(Key::Interrupt), Outcome::Cancelled);
}
