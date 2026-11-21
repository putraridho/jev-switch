//! A session search hit list as printed.

use crate::backend::ProviderId;
use crate::session::application::session_library::search::{project_name, Hit};
use crate::tui::render::Style;
use crate::tui::views::status_facts::startup::human_ago;

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
