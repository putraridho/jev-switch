use super::*;

#[test]
fn the_header_shows_each_subscription_and_what_it_has_left() {
    use crate::tui::app::screen_state::ProviderChip;
    let mut a = app();
    a.providers = vec![
        ProviderChip { name: "claude".into(), used: Some(32), cooling: None, via: None, credit: None },
        ProviderChip { name: "codex".into(), used: Some(85), cooling: None, via: None, credit: None },
        ProviderChip { name: "gemini".into(), used: None, cooling: Some("1h20m".into()), via: Some("agy".into()), credit: None },
        ProviderChip { name: "deepseek".into(), used: None, cooling: None, via: Some("deepseek".into()), credit: Some("$4.20 left".into()) },
    ];
    a.orchestrate = true;
    let header = render(&a, 140, Instant::now(), OFF).rows[0].clone();
    assert!(header.ends_with("⇄ auto · claude 32% · codex 85% · gemini (agy) cooling 1h20m · deepseek $4.20 left · orchestrate on"), "the CLI is named only when it differs: {header}");
    let on = render(&a, 140, Instant::now(), ON).rows[0].clone();
    assert!(on.contains("\x1b[33m85%"), "a plan window at 80% or more is yellow: {on:?}");
}

#[test]
fn the_header_names_the_branch_the_folder_and_the_context_window() {
    let mut a = app();
    a.git = Some("main ±3".into());
    a.session_name = Some("parser".into());
    a.context = Some(30);
    let header = render(&a, 140, Instant::now(), OFF).rows[0].clone();
    assert!(header.starts_with(" main ±3 /nonexistent \u{201c}parser\u{201d}"), "{header}");
    assert!(header.ends_with("  │  context 30%"), "{header}");
    a.context = Some(85);
    assert!(render(&a, 140, Instant::now(), OFF).rows[0].ends_with("context 85% · /compact"));
    // Narrow, the folder is cut before the router's side gives way.
    let narrow = render(&a, 80, Instant::now(), OFF).rows[0].clone();
    assert!(narrow.contains('…') && narrow.contains("⇄ auto"), "{narrow}");
    // A long folder gives way from its front; the session's name stays.
    a.place = "/private/var/folders/n0/j5fhc3d939n_f150v/T/jevux-rename/repo".into();
    a.context = None;
    let long = render(&a, 100, Instant::now(), OFF).rows[0].clone();
    assert!(long.contains(" …/") && long.contains("/jevux-rename/repo \u{201c}parser\u{201d}") && long.ends_with("⇄ auto · no provider ready · /doctor"), "{long}");
}
