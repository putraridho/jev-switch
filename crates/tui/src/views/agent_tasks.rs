//! The live task list: what the agent plans to do, ticking as it goes.

use crate::views::agent_render::{bar, clamp_width, fit};
use crate::render::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    InProgress,
    Done,
}

impl TaskState {
    fn glyph(self) -> &'static str {
        match self {
            Self::Pending => "☐",
            Self::InProgress => "▶",
            Self::Done => "☑",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub title: String,
    pub state: TaskState,
}

impl Task {
    pub fn new(title: &str, state: TaskState) -> Self {
        Self { title: title.into(), state }
    }
}

/// The list with a progress header. Done items are dimmed, the active one bold; the
/// glyph carries the state without colour. Empty list: `None` (nothing to show).
pub fn task_list(tasks: &[Task], width: usize, style: Style) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let width = clamp_width(width);
    let done = tasks.iter().filter(|t| t.state == TaskState::Done).count();
    let mut out = vec![format!("  {}  {}", style.bold("Tasks"), style.dim(&format!("{} {done} of {}", bar(done, tasks.len(), 8), tasks.len())))];
    for t in tasks {
        let line = format!("{} {}", t.state.glyph(), fit(&t.title, width.saturating_sub(6)));
        out.push(format!(
            "    {}",
            match t.state {
                TaskState::Done => style.dim(&line),
                TaskState::InProgress => style.bold(&line),
                TaskState::Pending => line,
            }
        ));
    }
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{display_width, strip_ansi};

    #[test]
    fn list_shows_progress_and_a_glyph_per_state() {
        let ts = [Task::new("read code", TaskState::Done), Task::new("write fix", TaskState::InProgress), Task::new("run tests", TaskState::Pending)];
        let t = task_list(&ts, 60, Style::off()).unwrap_or_default();
        let rows: Vec<&str> = t.lines().collect();
        assert!(rows[0].contains("1 of 3"));
        assert!(rows[1].contains("☑ read code") && rows[2].contains("▶ write fix") && rows[3].contains("☐ run tests"));
        assert!(!t.contains('\x1b'));
    }

    #[test]
    fn empty_list_shows_nothing_and_long_titles_fit() {
        assert_eq!(task_list(&[], 60, Style::off()), None);
        let long = Task::new(&"x".repeat(200), TaskState::Pending);
        for l in task_list(std::slice::from_ref(&long), 40, Style::off()).unwrap_or_default().lines() {
            assert!(display_width(l) <= 40);
        }
        let styled = task_list(std::slice::from_ref(&long), 40, Style::on()).unwrap_or_default();
        assert_eq!(strip_ansi(&styled), task_list(&[long], 40, Style::off()).unwrap_or_default());
    }
}
