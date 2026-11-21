use super::*;
use std::path::Path;
use crate::session::domain::saved_session::SessionInfo;
use crate::tui::render as uifmt;
use crate::tui::render::{ProviderStatus, Style};
use super::super::providers::avail_rows;
use super::super::startup::{banner, display_path, first_run_hint_once, human_ago, keep_tail, previous_session_line, resumed_line, FIRST_RUN_MARKER};

#[test]
fn the_home_directory_is_shown_as_a_tilde() {
    assert_eq!(display_path(Path::new("/home/me/proj"), Some("/home/me")), "~/proj");
    assert_eq!(display_path(Path::new("/home/me"), Some("/home/me/")), "~");
    assert_eq!(display_path(Path::new("/home/meagan/proj"), Some("/home/me")), "/home/meagan/proj");
    assert_eq!(display_path(Path::new("/srv/x"), None), "/srv/x");
}

#[test]
fn the_banner_is_one_line_naming_the_ready_providers_in_words() {
    let rows = avail_rows(&runtime(), NOW);
    let ready: Vec<String> = rows.iter().filter(|r| r.status == ProviderStatus::Ready && r.cooldown.is_none()).map(|r| r.provider.clone()).collect();
    let b = banner(Path::new("/home/me/proj"), Some("/home/me"), true, &rows, Style::off(), 100);
    let v = env!("CARGO_PKG_VERSION");
    assert!(b.starts_with(&format!("jev-code {v} · ")) && b.ends_with(" · ~/proj"), "{b}");
    assert_eq!(b.lines().count(), 1);
    if ready.is_empty() {
        assert!(b.contains("no provider ready · /doctor"), "{b}");
    } else {
        assert!(b.contains(&format!("{} ready", ready.join(", "))), "{b}");
    }
    assert!(!b.contains('●') && !b.contains('✕'), "no symbols to decode: {b}");
    let off = banner(Path::new("/x"), None, false, &rows, Style::off(), 100);
    assert!(off.ends_with("· Claude only · /x"), "{off}");
    let none = banner(Path::new("/x"), None, true, &[], Style::off(), 100);
    assert!(none.contains("no provider ready · /doctor · /x"), "{none}");
    let long = banner(Path::new(&format!("/{}", "deep/".repeat(40))), None, true, &rows, Style::off(), 60);
    assert!(long.contains(" ready") || long.contains("no provider ready"), "a long path never hides the providers: {long}");
    let narrow = banner(Path::new("/a/very/long/project/directory"), None, true, &rows, Style::off(), 40);
    assert!(uifmt::display_width(&narrow) <= 40, "{narrow}");
    // A path too long for the row loses its front, never the folder it names.
    let deep = banner(Path::new("/private/var/folders/n0/j5fhc3d939n_f150v/T/jevux-shot/repo"), None, true, &[], Style::off(), 80);
    assert!(deep.contains(" · …/") && deep.ends_with("/jevux-shot/repo") && uifmt::display_width(&deep) <= 80, "{deep}");
}

#[test]
fn a_path_is_cut_from_the_front_at_a_folder_boundary() {
    assert_eq!(keep_tail("~/proj", 20), "~/proj");
    assert_eq!(keep_tail("/a/b/c/project/src", 13), "…/project/src");
    assert_eq!(keep_tail("/a/b/c/project/src", 12), "…/src");
    assert_eq!(keep_tail("/averyveryverylongname", 8), "…ongname", "a last folder too long for the row keeps its end");
    assert_eq!(keep_tail("/a/b", 0), "…");
}

#[test]
fn ages_read_the_way_people_say_them() {
    for (ms, want) in [(0, "just now"), (59_999, "just now"), (60_000, "1m ago"), (59 * 60_000, "59m ago"), (60 * 60_000, "1h ago"), (47 * 3_600_000, "47h ago"), (48 * 3_600_000, "2d ago")] {
        assert_eq!(human_ago(ms), want, "{ms}");
    }
}

#[test]
fn the_previous_session_line_offers_resume() {
    let info = SessionInfo { turns: 12, last_used_ms: NOW - 2 * 3_600_000, providers: vec![Claude], current: Some((Claude, "sonnet".into())) };
    assert_eq!(previous_session_line(&info, NOW, Style::off()), "previous session: 12 turns, 2h ago · /resume");
    let one = SessionInfo { turns: 1, ..info.clone() };
    assert!(previous_session_line(&one, NOW, Style::off()).contains("1 turn,"));
    assert_eq!(resumed_line(&info, NOW, Style::off()), "✓ resumed 12 turns, last on claude · sonnet, 2h ago");
}

fn temp_dir(name: &str) -> PathBuf {
    crate::testkit::scratch_dir::scratch_dir("views", name)
}

#[test]
fn the_first_run_hint_is_shown_once_per_machine() {
    let dir = temp_dir("first-run");
    let one_ready = vec![av(Claude, State::Ready), av(Codex, State::NotInstalled)];
    let hint = first_run_hint_once(&one_ready, Some(&dir)).expect("first time");
    assert!(hint.contains("Only one provider is ready"), "{hint}");
    assert!(dir.join(FIRST_RUN_MARKER).exists());
    assert_eq!(first_run_hint_once(&one_ready, Some(&dir)), None, "never again");

    let other = temp_dir("first-run-fine");
    let all_ready = vec![av(Claude, State::Ready), av(Codex, State::Ready)];
    assert_eq!(first_run_hint_once(&all_ready, Some(&other)), None, "nothing to say");
    assert!(!other.join(FIRST_RUN_MARKER).exists(), "the marker is only written after a hint was shown");

    assert_eq!(first_run_hint_once(&one_ready, None), None, "no state directory: no nagging");
    let blocked = dir.join("nope").join("deeper");
    assert_eq!(first_run_hint_once(&one_ready, Some(&blocked)), None, "a marker that cannot be written means no hint");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(other);
}
