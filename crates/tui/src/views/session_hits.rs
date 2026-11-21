//! A session search hit list as printed.

use jev_backend::ProviderId;
use jev_session::application::session_library::search::{project_name, Hit};
use crate::render::Style;
use crate::views::status_facts::startup::human_ago;

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

#[cfg(test)]
mod tests {
    use super::*;
    use jev_session::application::session_library::query::Query;
    use jev_session::application::session_library::scan_all;
    use jev_session::application::session_library::search::search;
    use jev_session::infrastructure::library_fixtures::{library, NOW};
    use jev_session::infrastructure::session_file::{self, FileSessions};

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
}
