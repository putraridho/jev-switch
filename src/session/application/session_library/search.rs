//! Searching the library and printing a hit list.

use std::path::Path;

use super::query::Query;
use super::*;
use crate::backend::ProviderId;
use crate::tui::render::Style;
use crate::tui::views::status_facts::human_ago;

/// One session that answers a query, and the line that matched its words.
#[derive(Debug, Clone)]
pub struct Hit {
    pub entry: LibraryEntry,
    pub snippet: Option<String>,
}

/// The sessions that answer `query`: pinned first, then newest first. With words, only
/// sessions whose text holds every word, each with the first line that held one.
pub fn search(entries: Vec<LibraryEntry>, query: &Query, now_ms: u64) -> Vec<Hit> {
    let mut hits: Vec<Hit> = entries
        .into_iter()
        .filter(|e| query.admits(e, now_ms))
        .filter_map(|entry| {
            if query.words.is_empty() {
                return Some(Hit { entry, snippet: None });
            }
            let lines = entry.haystack();
            let everywhere = query.words.iter().all(|w| lines.iter().any(|l| l.contains(w.as_str())));
            let snippet = everywhere.then(|| snippet_for(&entry, &query.words[0])).flatten();
            everywhere.then_some(Hit { entry, snippet })
        })
        .collect();
    hits.sort_by_key(|h| !h.entry.meta.pinned);
    hits
}

/// The first original line holding `word`, shortened around it.
fn snippet_for(e: &LibraryEntry, word: &str) -> Option<String> {
    let log = e.saved.sessions.log();
    let lines = log.iter().flat_map(|t| t.prompt.lines().chain(t.reply.lines()).chain(t.tools.iter().map(String::as_str)));
    let line = lines.into_iter().find(|l| l.to_lowercase().contains(word))?.trim().to_string();
    // Lowercasing can turn one character into several (U+0130 becomes `i` and a combining dot),
    // so neither byte nor character offsets carry over. The word is found in the whole-string
    // lowering (the one the match used: a word-final Σ becomes ς there), and mapped back with a
    // table built char by char of where each lowered byte's character starts in `line`. The only
    // mapping that depends on context is σ/ς, both two bytes, so the two lowerings line up.
    let lowered = line.to_lowercase();
    let mut origin = Vec::with_capacity(lowered.len());
    for (i, c) in line.char_indices() {
        let width: usize = c.to_lowercase().map(char::len_utf8).sum();
        origin.extend(std::iter::repeat_n(i, width));
    }
    let at = lowered.find(word).and_then(|found| origin.get(found).copied()).unwrap_or(0);
    let start = line[..at].char_indices().rev().nth(30).map_or(0, |(i, _)| i);
    let chars: String = line[start..].chars().take(90).collect();
    Some(if start > 0 { format!("...{chars}") } else { chars })
}

/// A hit list as printed: a number, the title, then marks, age and the matched line.
pub fn render_hits(hits: &[Hit], now_ms: u64, current_project: &str, style: Style) -> Vec<String> {
    hits.iter()
        .enumerate()
        .flat_map(|(n, h)| {
            let e = &h.entry;
            let pin = if e.meta.pinned { style.yellow("★ ") } else { String::new() };
            let title: String = e.title().chars().take(70).collect();
            let turns = if e.saved.sessions.turns() == 1 { "1 turn".to_string() } else { format!("{} turns", e.saved.sessions.turns()) };
            let mut facts = vec![human_ago(now_ms.saturating_sub(e.saved.saved_ms)), turns];
            let providers: Vec<&str> = e.saved.sessions.providers_used().iter().map(ProviderId::as_str).collect();
            facts.push(providers.join("+"));
            if e.saved.project != current_project {
                facts.push(project_name(&e.saved.project));
            }
            facts.extend(e.meta.tags.iter().map(|t| format!("#{t}")));
            if e.meta.archived {
                facts.push("archived".to_string());
            }
            let mut rows = vec![format!("{:>3}. {pin}{title}", n + 1), format!("     {}", style.dim(&facts.join(" · ")))];
            if let Some(s) = &h.snippet {
                rows.push(format!("     {}", style.dim(&format!("\u{201c}{s}\u{201d}"))));
            }
            rows
        })
        .collect()
}

/// The last folder of a project path.
pub fn project_name(project: &str) -> String {
    Path::new(project).file_name().map_or_else(|| project.to_string(), |n| n.to_string_lossy().into_owned())
}
