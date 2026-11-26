//! Shell leaf modules: autostart, tray icon, CLI, local time.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use jev_dashboard::{autostart, cli, tray_icon};
use jev_usage_core::assess::Severity;

#[test]
fn desktop_entry_quotes_the_executable() {
    let entry = autostart::desktop_entry(r#"/opt/Flashboard "$HOME/bin"/jev-dashboard"#);
    assert!(entry.contains("Name=Flashboard"));
    assert!(entry.contains("--hidden"));
    // `"`, `$` and `\` are backslash-escaped inside the double quotes.
    assert!(entry.contains("Exec=\""));
    assert!(entry.contains("\\\""));
    assert!(entry.contains("\\$"));
}

#[test]
fn cli_parses_hidden_anywhere() {
    assert!(!cli::parse([].into_iter()).hidden);
    assert!(cli::parse(["--hidden".to_owned()].into_iter()).hidden);
    assert!(cli::parse(["x".to_owned(), "--hidden".to_owned()].into_iter()).hidden);
    assert!(!cli::parse(["--Hidden".to_owned()].into_iter()).hidden);
}

#[test]
fn tray_icon_render_is_deterministic_and_sized() {
    for step in [0_u8, 10, 20] {
        for level in [
            Severity::Ok,
            Severity::Warn,
            Severity::Critical,
            Severity::Unknown,
        ] {
            let spec = tray_icon::IconSpec {
                used_step: step,
                level,
            };
            let a = tray_icon::render(spec, 64);
            let b = tray_icon::render(spec, 64);
            assert_eq!(a.len(), 64 * 64 * 4);
            assert_eq!(a, b);
        }
    }
}
