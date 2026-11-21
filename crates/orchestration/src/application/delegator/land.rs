//! Landing a finished delegation in the main tree, once nobody is editing it.

use crate::domain::delegation::{DelegateRequest, DelegateResult, Landing, ReviewReason};
use crate::domain::file_patch::{first_page, split_patch, MAX_DIFF_CHARS};

use super::outcome::{Finished, Pending, Verdict};
use super::{lock, Applied, Delegator};

impl Delegator {
    /// Land a finished delegation in the main tree: a worker whose tests passed is applied
    /// (or goes to review when its patch conflicts); one under review gets its first diff
    /// page. Called when the planner is not editing: inside `delegate` or `wait`.
    pub(super) async fn settle(&self, finished: Finished) -> DelegateResult {
        let pending = match finished {
            Finished::Done(result) => return *result,
            Finished::Pending(pending) => *pending,
        };
        let Pending { mut result, patch, verdict } = pending;
        let reason = match verdict {
            Verdict::Review(reason) => reason,
            Verdict::Passed { command } => match self.auto_apply(&result.worker_id, &patch, result.files_changed()).await {
                Ok(()) => {
                    // A worktree that will not go is reported by `forget` and cleaned by recovery later.
                    let _ = self.forget(&result.worker_id).await;
                    result.landing = Landing::AutoMerged { command };
                    return result;
                }
                Err(why) => ReviewReason::Conflict(why),
            },
        };
        let (diff, shown) = first_page(&split_patch(&patch), MAX_DIFF_CHARS);
        self.update(&result.worker_id, |w| w.shown.extend(shown.iter().cloned())).await;
        result.diff = diff;
        result.shown = shown;
        result.landing = Landing::Review(reason);
        result
    }

    /// Apply a tested worker's patch to the main tree, one change at a time. Recorded for
    /// `undo` in the same blocking step as the apply, so a call cancelled meanwhile still
    /// leaves its changes undoable.
    pub(super) async fn auto_apply(&self, id: &str, patch: &str, files: Vec<String>) -> Result<(), String> {
        let _turn = self.main_tree.lock().await;
        let (merged, id, applied) = (self.merged.clone(), id.to_string(), Applied { patch: patch.to_string(), files });
        self.blocking(move |w| {
            w.apply(&applied.patch)?;
            lock(&merged).insert(id, applied);
            Ok(())
        })
        .await
        .map_err(|e| format!("{e:#}"))
    }

    /// Run a delegation to the end and land it: what a blocking call returns.
    pub async fn delegate(&self, req: DelegateRequest) -> DelegateResult {
        let finished = self.run(req).await;
        self.settle(finished).await
    }
}
