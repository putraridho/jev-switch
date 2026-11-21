//! `/diff`, `/undo` and `/rewind`: what the last turns changed.

use jev_changes::application::undo_turn::undo;
use jev_changes::application::ports::WorkingTree;
use std::sync::Arc;
use jev_changes::domain::change_set::ChangeSummary;
use jev_changes::domain::undo_report::UndoError;
use jev_changes::domain::undo_report::UndoReport;
use super::Engine;

impl Engine {
    /// The working tree the changes of each turn are read from. Without one nothing is
    /// tracked, so `/diff` and `/undo` have nothing to work on.
    pub fn with_working_tree(mut self, tree: Arc<dyn WorkingTree>) -> Self {
        self.working_tree = Some(tree);
        self
    }

    /// What the last turn changed, if it changed anything in a git working tree.
    pub fn last_changes(&self) -> Option<&ChangeSummary> {
        self.last_changes.as_ref()
    }

    /// The turns that changed files, oldest first.
    pub fn change_history(&self) -> &[ChangeSummary] {
        &self.change_history
    }

    /// Put the working tree back to how it was before checkpoint `number` (1-based, as
    /// `/diff --turn` counts), undoing the newer checkpoints first. Stops at the first
    /// one that cannot be undone and reports how far it got.
    pub fn rewind_to(&mut self, number: usize) -> Result<Vec<UndoReport>, (Vec<UndoReport>, UndoError)> {
        if number == 0 || number > self.change_history.len() {
            return Err((Vec::new(), UndoError::NothingToUndo));
        }
        let mut done = Vec::new();
        while self.change_history.len() >= number {
            // The newest checkpoint is the one `undo_last` works on.
            self.last_changes = self.change_history.last().cloned();
            match self.undo_last() {
                Ok(r) => done.push(r),
                Err(e) => return Err((done, e)),
            }
        }
        self.last_changes = self.change_history.last().cloned();
        Ok(done)
    }

    /// The user changed the working tree on purpose (a hunk reverted in `/diff --review`):
    /// record the tree as it is now as where the newest checkpoint ended, so `/undo` and
    /// `/rewind` keep working instead of reading the change as someone else's edit.
    /// Returns false when there is no checkpoint or the tree cannot be read.
    pub fn adopt_working_tree(&mut self) -> bool {
        let Some(tree) = &self.working_tree else { return false };
        let Some(now) = tree.snapshot(&self.dir) else { return false };
        let Some(newest) = self.change_history.last_mut() else { return false };
        newest.after = now.tree.clone();
        // Untracked files the turn added: follow their new look, drop the ones now gone.
        newest.added_stamps = newest.added_stamps.keys().filter_map(|k| now.untracked.get(k).map(|s| (k.clone(), *s))).collect();
        if self.last_changes.is_some() {
            self.last_changes = Some(newest.clone());
        }
        self.save_quietly();
        true
    }

    /// The text diff of the last turn, whole or for one repository-relative path; plain
    /// text, for the UI to colour. `None` when there is no summary or git fails.
    pub fn last_diff(&self, path: Option<&str>) -> Option<String> {
        let c = self.last_changes.as_ref()?;
        self.working_tree.as_ref()?.diff_text(&self.dir, c, path)
    }

    /// Put back what the last turn changed, and only that, if nothing it touched was
    /// edited since (see [`undo`]). One level: the summary is consumed on
    /// success. The conversation is told, so the next catch-up is truthful. Blocking
    /// (a few git runs), like any user command.
    pub fn undo_last(&mut self) -> Result<UndoReport, UndoError> {
        let summary = self.last_changes.clone().ok_or(UndoError::NothingToUndo)?;
        let tree = self.working_tree.as_ref().ok_or_else(|| UndoError::Failed("changes are not tracked".into()))?;
        let report = undo(tree.as_ref(), &self.dir, &summary)?;
        self.last_changes = None;
        self.change_history.pop();
        self.sessions.record_undo();
        self.save_quietly();
        Ok(report)
    }
}
