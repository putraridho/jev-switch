use super::marks::{mark, Mark};
use super::query::Query;
use super::search::search;
use crate::tui::render::Style;
use crate::tui::views::session_hits::render_hits;
use super::*;

#[test]
fn search_finds_words_in_replies_across_projects_and_shows_the_line() {
    let (state, ..) = library("words");
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("parser").unwrap(), NOW);
    assert_eq!(titles(&hits), ["parser bug", "write docs"]);
    assert!(hits[1].snippet.as_deref().unwrap().contains("Docs written about the parser"), "{:?}", hits[1].snippet);
    assert!(search(scan_all(&FileSessions, &state), &Query::parse("parser lexer").unwrap(), NOW).is_empty());
}

#[test]
fn a_line_whose_lowercase_is_longer_does_not_break_the_snippet() {
    // U+0130 lowercases to three bytes, so an offset found in the copy is not one of the line.
    let state = temp("fold");
    let proj = state.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    put(&state, &proj, 1, vec![turn(ProviderId::Claude, "ask", "\u{130}\u{130}\u{130} foo is here", &[])]);
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("foo").unwrap(), NOW);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.as_deref().unwrap().contains("foo"), "{:?}", hits[0].snippet);
    // Longer than the lead-in, so a character count off by one per U+0130 would skip past it.
    let long = format!("{} foo {}", "\u{130}".repeat(40), "z".repeat(200));
    put(&state, &proj, 2, vec![turn(ProviderId::Claude, "ask", &long, &[])]);
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("foo").unwrap(), NOW);
    assert!(hits.iter().all(|h| h.snippet.as_deref().unwrap().contains("foo")), "{:?}", hits.iter().map(|h| &h.snippet).collect::<Vec<_>>());
}

#[test]
fn a_word_ending_in_capital_sigma_is_found_in_the_snippet() {
    let state = temp("sigma");
    let proj = state.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    let line = format!("{} ΟΔΟΣ {}", "a".repeat(60), "b".repeat(100));
    put(&state, &proj, 1, vec![turn(ProviderId::Claude, "ask", &line, &[])]);
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("ΟΔΟΣ").unwrap(), NOW);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.as_deref().unwrap().contains("ΟΔΟΣ"), "{:?}", hits[0].snippet);
    // Not final before an apostrophe and a letter (σ), so only the later one matches.
    let line = format!("ΟΔΟΣ'Α {} ΟΔΟΣ end", "a".repeat(100));
    put(&state, &proj, 2, vec![turn(ProviderId::Claude, "ask", &line, &[])]);
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("ΟΔΟΣ").unwrap(), NOW);
    let later: Vec<_> = hits.iter().filter_map(|h| h.snippet.as_deref()).filter(|s| s.contains("end")).collect();
    assert!(later.len() == 1 && later[0].starts_with("..."), "{later:?}");
}

#[test]
fn filters_narrow_by_provider_file_date_and_project() {
    let (state, ..) = library("filters");
    let run = |q: &str| titles(&search(scan_all(&FileSessions, &state), &Query::parse(q).unwrap(), NOW));
    assert_eq!(run("provider:codex"), ["parser bug", "write docs"]);
    assert_eq!(run("file:lexer.rs"), ["fix the lexer"]);
    assert_eq!(run("since:7d"), ["fix the lexer", "parser bug"]);
    assert_eq!(run("project:proj-b"), ["parser bug"]);
    assert_eq!(run("provider:codex since:1w parser"), ["parser bug"]);
    assert!(Query::parse("provider:nobody").is_err());
    assert!(Query::parse("since:soon").is_err());
    for odd in ["since:7日", "since:3д", "since:é", "since:"] {
        assert!(Query::parse(odd).is_err(), "{odd}");
    }
}

#[test]
fn enormous_since_filters_include_all_sessions() {
    let (state, ..) = library("huge-since");
    for filter in ["since:99999999999999d", "since:18446744073709551615d", "since:99999999999999w", "since:18446744073709551615w"] {
        let query = Query::parse(filter).unwrap();
        assert_eq!(query.since_ms, Some(u64::MAX));
        assert_eq!(search(scan_all(&FileSessions, &state), &query, NOW).len(), 3);
    }
}

#[test]
fn pinned_come_first_archived_hide_and_tags_filter() {
    let (state, ..) = library("marks");
    let all = scan_all(&FileSessions, &state);
    let docs = all.iter().find(|e| e.title() == "write docs").unwrap().path.clone();
    let lexer = all.iter().find(|e| e.title() == "fix the lexer").unwrap().path.clone();
    mark(&FileSessions, &state, &docs, &Mark::Pin(true)).unwrap();
    mark(&FileSessions, &state, &lexer, &Mark::Archive(true)).unwrap();
    mark(&FileSessions, &state, &docs, &Mark::Tag("docs".into())).unwrap();
    let run = |q: &str| titles(&search(scan_all(&FileSessions, &state), &Query::parse(q).unwrap(), NOW));
    assert_eq!(run(""), ["write docs", "parser bug"]);
    assert_eq!(run("archived"), ["fix the lexer"]);
    assert_eq!(run("all").len(), 3);
    assert_eq!(run("tag:docs"), ["write docs"]);
    assert_eq!(run("pinned"), ["write docs"]);
    mark(&FileSessions, &state, &docs, &Mark::Untag("docs".into())).unwrap();
    assert!(run("tag:docs").is_empty());
}

#[test]
fn rendered_hits_show_marks_and_the_matching_line() {
    let (state, a, _) = library("render");
    let hits = search(scan_all(&FileSessions, &state), &Query::parse("lexer").unwrap(), NOW);
    let text = render_hits(&hits, NOW, &session_file::project_key(&a), Style::off()).join("\n");
    assert!(text.contains("  1. fix the lexer") && text.contains("24h ago") && text.contains("claude"), "{text}");
    assert!(!text.contains("proj-a"), "the current project is not repeated: {text}");
    let elsewhere = render_hits(&hits, NOW, "/other", Style::off()).join("\n");
    assert!(elsewhere.contains("proj-a"), "{elsewhere}");
}
