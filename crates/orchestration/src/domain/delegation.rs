//! What the planner asks for and what comes back: the request, the result, and how a
//! finished worker's work landed.

use std::time::Duration;

use jev_backend::ProviderId;

use super::file_patch::FileStat;

/// What the planner asks for. There is deliberately no model field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegateRequest {
    pub brief: String,
    /// Hints about where the work is; not limits.
    pub files: Vec<String>,
    pub read_only: bool,
    /// The planner wants to review the diff itself even if the tests pass.
    pub review: bool,
}

/// Why a worker's changes were not merged on their own and wait for the planner's review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewReason {
    /// The planner asked to review.
    Asked,
    /// Auto-merge is switched off (`JEV_AUTO_MERGE=0`).
    Off,
    /// The project has no test command jev knows, so nothing proved the work.
    NoTestCommand,
    TestsFailed { command: String, output: String },
    TestsTimedOut { command: String, after: Duration },
    TestsNotRun { command: String, error: String },
    /// The tests passed but the patch does not apply cleanly to the user's tree.
    Conflict(String),
}

impl ReviewReason {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Asked => "review_asked",
            Self::Off => "review_off",
            Self::NoTestCommand => "review_no_tests",
            Self::TestsFailed { .. } => "review_tests_failed",
            Self::TestsTimedOut { .. } => "review_tests_timed_out",
            Self::TestsNotRun { .. } => "review_tests_not_run",
            Self::Conflict(_) => "review_conflict",
        }
    }
}

impl std::fmt::Display for ReviewReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Asked => f.write_str("you asked to review it"),
            Self::Off => f.write_str("auto-merge is off (JEV_AUTO_MERGE=0)"),
            Self::NoTestCommand => f.write_str("this project has no test command jev knows (Cargo, npm, Python or Go), so nothing proved the work"),
            Self::TestsFailed { command, output } => write!(f, "`{command}` failed in the worker's worktree. The end of its output:\n{output}"),
            Self::TestsTimedOut { command, after } => write!(f, "`{command}` ran longer than {} minutes in the worker's worktree and was stopped", after.as_secs() / 60),
            Self::TestsNotRun { command, error } => write!(f, "`{command}` could not be run: {error}"),
            Self::Conflict(why) => write!(f, "the tests passed, but the changes do not apply cleanly to your working tree: {why}"),
        }
    }
}

/// How a successful worker's work reached the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Landing {
    /// A read-only worker: a report and nothing to merge. `discarded` lists files it changed
    /// anyway in its throwaway worktree (empty when it ran in place or kept its word).
    Report { discarded: Vec<String> },
    /// It finished without changing any file.
    NoChanges,
    /// Its tests passed and its changes are in the user's working tree; `undo` reverses them.
    AutoMerged { command: String },
    /// Its worktree is kept for the planner to review, then merge or discard.
    Review(ReviewReason),
}

impl Landing {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Report { .. } => "report",
            Self::NoChanges => "no_changes",
            Self::AutoMerged { .. } => "auto_merged",
            Self::Review(r) => r.tag(),
        }
    }
}

/// What comes back to the planner.
#[derive(Debug, Clone)]
pub struct DelegateResult {
    pub worker_id: String,
    pub provider: Option<ProviderId>,
    pub model: Option<String>,
    /// The worker's final text, truncated.
    pub summary: String,
    /// EVERY file the worker changed, with its +/- counts, never cut short.
    pub stats: Vec<FileStat>,
    /// The diffs of the files in `shown`, whole files only, up to [`MAX_DIFF_CHARS`](super::file_patch::MAX_DIFF_CHARS).
    pub diff: String,
    /// The files whose full diff is in `diff`. The rest need `show_diff` before a merge.
    pub shown: Vec<String>,
    pub ok: bool,
    pub error: Option<String>,
    /// What happened to earlier attempts (a limit hit, a refused model).
    pub retries: Vec<String>,
    /// Where the work went. Only meaningful when `ok`.
    pub landing: Landing,
}

impl DelegateResult {
    pub fn failed(error: String, retries: Vec<String>) -> Self {
        Self {
            worker_id: String::new(),
            provider: None,
            model: None,
            summary: String::new(),
            stats: vec![],
            diff: String::new(),
            shown: vec![],
            ok: false,
            error: Some(error),
            retries,
            landing: Landing::NoChanges,
        }
    }

    pub fn files_changed(&self) -> Vec<String> {
        self.stats.iter().map(|s| s.path.clone()).collect()
    }

    /// The text the planner reads.
    pub fn render(&self) -> String {
        let who = match (&self.provider, &self.model) {
            (Some(p), Some(m)) => format!("{p} {m}"),
            _ => "no worker".to_string(),
        };
        let status = match (self.ok, &self.landing) {
            (false, _) => "FAILED",
            (true, Landing::Report { .. }) => "done (read-only)",
            (true, Landing::AutoMerged { .. }) => "done, auto-merged",
            (true, Landing::NoChanges) => "done, no changes",
            (true, Landing::Review(_)) => "done, waiting for your review",
        };
        let mut out = vec![format!("worker {} ({who}): {status}", self.worker_id)];
        out.extend(self.retries.iter().map(|r| format!("retry: {r}")));
        if let Some(e) = &self.error {
            out.push(format!("error: {e}"));
        }
        if !self.ok {
            return out.join("\n");
        }
        let id = &self.worker_id;
        let reason = match &self.landing {
            Landing::Report { discarded } => {
                out.push(format!("worker report:\n{}", self.summary));
                if !discarded.is_empty() {
                    out.push(format!("It changed files although the piece was read-only; they were thrown away: {}.", discarded.join(", ")));
                }
                return out.join("\n");
            }
            Landing::NoChanges => {
                out.push("files changed: none".into());
                out.push(format!("worker report:\n{}", self.summary));
                return out.join("\n");
            }
            Landing::AutoMerged { command } => {
                self.push_files_and_report(&mut out);
                out.push(format!(
                    "Auto-merged into your working tree: `{command}` passed in the worker's worktree and the changes applied cleanly. \
                     undo({id}) takes exactly these changes back out."
                ));
                return out.join("\n");
            }
            Landing::Review(reason) => reason,
        };
        self.push_files_and_report(&mut out);
        out.push(format!("Not auto-merged: {reason}"));
        let shown = if self.shown.is_empty() { "no file".to_string() } else { self.shown.join(", ") };
        out.push(format!("diff (full diffs of: {shown}):\n{}", if self.diff.is_empty() { "(empty)" } else { &self.diff }));
        let unseen: Vec<&str> = self.stats.iter().map(|s| s.path.as_str()).filter(|p| !self.shown.iter().any(|s| s == p)).collect();
        if !unseen.is_empty() {
            out.push(format!(
                "NOT SHOWN here (too long): {}. Call show_diff({id}, file) for each to read it; merge refuses files you have not seen in full.",
                unseen.join(", ")
            ));
        }
        out.push(format!("Review the diff, then merge({id}) to apply it (or merge({id}, files) for some files) or discard({id}) to drop it."));
        out.join("\n")
    }

    fn push_files_and_report(&self, out: &mut Vec<String>) {
        out.push(format!("files changed ({}):", self.stats.len()));
        out.extend(self.stats.iter().map(|s| format!("  {}", s.render())));
        out.push(format!("worker report:\n{}", self.summary));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_text_names_the_worker_and_failure() {
        let failed = DelegateResult::failed("boom".into(), vec!["claude hit its limit".into()]);
        let text = failed.render();
        assert!(text.contains("FAILED") && text.contains("error: boom") && text.contains("retry: claude hit its limit"));
    }

}
