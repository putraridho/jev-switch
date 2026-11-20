//! `/undo`: revert the last turn's changes, after asking.

use crate::changes::infrastructure::git_working_tree::UndoError;
use crate::tui::render as uifmt;
use crate::tui::repl::session::{Io, Repl};
use crate::tui::views::status_facts as views;

impl Repl {
    pub(super) async fn undo(&mut self, io: &mut Io<'_>) {
        let style = self.out_style();
        let Some(summary) = self.engine.last_changes().cloned() else {
            self.console.out(&format!("  {}", style.dim("Nothing to undo: the last turn changed no files.")));
            return;
        };
        if let Some(m) = &summary.outside.moved {
            // The repository moved during the turn: refuse before listing anything to revert.
            let e = UndoError::RepoMoved(format!("the repository moved during the turn ({})", m.describe()));
            self.console.out(&uifmt::problem_block("Not undone", &[e.to_string()], &[], style));
            return;
        }
        if summary.files.is_empty() {
            self.console.out(&format!("  {}", style.dim("Nothing to undo: the last turn's own tools changed no files.")));
            return;
        }
        if let Some(w) = summary.outside.warning() {
            self.console.out(&format!("  {} {}", style.yellow("⚠"), w));
        }
        let files = views::file_changes(&summary);
        let noun = if files.len() == 1 { "file" } else { "files" };
        self.console.out(&format!("  Would revert {} {noun}:", files.len()));
        self.console.out(&uifmt::changes_block(&files, style, self.console.width()));
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!(
                "{} Not undone: asking needs a terminal. Run /undo in an interactive session.",
                style.yellow("⚠")
            ));
            return;
        };
        if !confirm.confirm(&format!("Revert {} {noun}? [y/N] ", files.len())) {
            self.console.out(&format!("  {}", style.dim("Left as it is.")));
            return;
        }
        match self.engine.undo_last() {
            Ok(r) => {
                let mut parts: Vec<String> = Vec::new();
                for (verb, list) in [("restored", &r.restored), ("removed", &r.removed), ("recreated", &r.recreated)] {
                    if !list.is_empty() {
                        parts.push(format!("{verb} {}", list.join(", ")));
                    }
                }
                self.console.out(&format!("{} reverted: {}", style.green("✓"), parts.join("; ")));
            }
            Err(UndoError::ChangedSince(changed)) => {
                let steps = [
                    "Look at what differs: `/diff`".to_string(),
                    "Revert by hand (for example `git checkout -- <file>`), or commit your edits first".to_string(),
                ];
                self.console.out(&uifmt::problem_block("Not undone: those files changed since the turn", &changed, &steps, style));
            }
            Err(e) => self.console.out(&uifmt::problem_block("Not undone", &[e.to_string()], &[], style)),
        }
    }
}
