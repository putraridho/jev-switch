use super::*;
use jev_backend::infrastructure::mcp_discovery::McpTransport;
use jev_backend::infrastructure::mcp_server::group;

use crate::render::display_width;

#[test]
fn one_row_per_server_with_its_providers() {
    let t = text(&McpPicker::new(servers(), 10));
    assert!(t.contains("❯ ◌ luna"), "{t}");
    assert!(t.contains("http · claude · codex"), "{t}");
    assert!(t.contains("stdio · claude"), "{t}");
    assert_eq!(t.matches("luna").count(), 1, "{t}");
    assert!(t.contains("checking servers…"), "{t}");
}

#[test]
fn answers_fill_in_the_dots() {
    let mut p = McpPicker::new(servers(), 10);
    p.set_health("luna", McpScope::Claude, McpHealth::Connected);
    p.set_health("luna", McpScope::Codex, McpHealth::Unknown);
    p.set_health(
        "pw",
        McpScope::Claude,
        McpHealth::Failed("spawn npx ENOENT".into()),
    );
    p.set_health("posthog", McpScope::Claude, McpHealth::NeedsLogin);
    p.set_health("nobody", McpScope::Claude, McpHealth::Connected);
    let t = text(&p);
    assert!(t.contains("● luna"), "{t}");
    assert!(t.contains("✗ pw") && t.contains("failed"), "{t}");
    assert!(t.contains("◐ posthog") && t.contains("needs login"), "{t}");
    assert!(!t.contains("checking servers…"), "{t}");
}

#[test]
fn a_stdio_server_has_no_login() {
    let mut p = McpPicker::new(servers(), 10);
    p.press(Key::Down);
    p.press(Key::Enter);
    let t = text(&p);
    assert!(
        t.contains("No login needed") && t.contains("❯ Back") && !t.contains("Log in"),
        "{t}"
    );
}

#[test]
fn the_card_says_why_a_provider_failed() {
    let mut p = McpPicker::new(servers(), 10);
    p.set_health(
        "luna",
        McpScope::Codex,
        McpHealth::Failed("connection refused".into()),
    );
    p.press(Key::Enter);
    let t = text(&p);
    assert!(t.contains("◌ claude  checking"), "{t}");
    assert!(t.contains("✗ codex   failed — connection refused"), "{t}");
}

#[test]
fn hostile_names_cannot_drive_the_terminal_or_overflow() {
    let long = format!("go\x1b[2J{}", "x".repeat(200));
    let mut rows = vec![row(&long, McpScope::Claude, McpTransport::Http)];
    rows[0].target = format!("https://\x1b[31m{}", "y".repeat(300));
    let mut p = McpPicker::new(group(&rows), 10);
    p.set_health(
        &long,
        McpScope::Claude,
        McpHealth::Failed(format!("bad\x1b]0;title\x07{}", "z".repeat(300))),
    );
    for width in [24, 40, 70] {
        let lines = p.render(width, Style::off());
        assert!(
            lines
                .iter()
                .all(|l| !l.contains('\x1b') && display_width(l) <= width),
            "{width}: {lines:?}"
        );
    }
    p.press(Key::Enter);
    for width in [24, 40, 70] {
        let lines = p.render(width, Style::off());
        assert!(
            lines
                .iter()
                .all(|l| !l.contains('\x1b') && display_width(l) <= width),
            "{width}: {lines:?}"
        );
    }
}
