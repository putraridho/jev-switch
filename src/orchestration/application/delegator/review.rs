//! The planner's side of a worker that waits for review: page its diff, merge, discard, undo.

use crate::orchestration::domain::file_patch::{char_page, join_diffs, FileDiff, MAX_DIFF_CHARS, split_patch};
use crate::orchestration::domain::worker::Worker;

use super::{lock, Delegator};

impl Delegator {
    /// A finished worker, and its diff split per file (files already merged left out).
    pub(super) async fn finished_diff(&self, id: &str) -> Result<(Worker, Vec<FileDiff>), String> {
        let worker = self.lookup(id)?;
        let path = match (&worker.path, worker.running) {
            (Some(path), false) => path.clone(),
            _ => return Err(format!("worker {id} is still running")),
        };
        let diff = self.blocking(move |w| w.diff(&path)).await.map_err(|e| format!("could not read worker {id}'s changes: {e:#}"))?;
        let parts = split_patch(&diff).into_iter().filter(|p| !worker.merged.contains(&p.path)).collect();
        Ok((worker, parts))
    }

    pub async fn show_diff(&self, id: &str, file: Option<&str>, offset: usize) -> Result<String, String> {
        let (_, parts) = self.finished_diff(id).await?;
        if parts.is_empty() {
            return Ok(format!("worker {id} has no changes left to show"));
        }
        let all: Vec<String> = parts.iter().map(|p| p.path.clone()).collect();
        // The text to page, and where each file sits in it: (path, start, length) in characters.
        let (text, spans): (String, Vec<(String, usize, usize)>) = match file {
            Some(f) => {
                let part = parts.iter().find(|p| p.path == f).ok_or_else(|| format!("worker {id} changed no file `{f}`; files: {}", all.join(", ")))?;
                (part.text.clone(), vec![(part.path.clone(), 0, part.chars())])
            }
            None => {
                let mut at = 0;
                let spans = parts
                    .iter()
                    .map(|p| {
                        let start = at;
                        at += p.chars();
                        (p.path.clone(), start, p.chars())
                    })
                    .collect();
                (join_diffs(&parts), spans)
            }
        };
        let total = text.chars().count();
        if offset >= total {
            return Err(format!("offset {offset} is past the end ({total} characters)"));
        }
        let (page, end) = char_page(&text, offset, MAX_DIFF_CHARS);
        self.update(id, |w| {
            for (path, start, len) in &spans {
                let (lo, hi) = (offset.max(*start), end.min(start + len));
                if lo < hi {
                    w.cover(path, lo - start, hi - start, *len);
                }
            }
        })
        .await;
        let seen = self.lookup(id)?.shown;
        let unseen: Vec<&str> = all.iter().map(String::as_str).filter(|p| !seen.contains(*p)).collect();
        let what = file.map_or("all files".to_string(), |f| format!("file {f}"));
        let more = if end < total {
            let scope = file.map(|f| format!(", file \"{f}\"")).unwrap_or_default();
            format!("[more: call show_diff with worker_id \"{id}\"{scope}, offset {end}]")
        } else {
            "[end]".to_string()
        };
        let seen_line = if unseen.is_empty() { "Every file has now been shown in full.".to_string() } else { format!("Not yet shown in full: {}.", unseen.join(", ")) };
        Ok(format!("worker {id}, {what}, characters {offset}-{end} of {total}:\n{page}\n{more}\n{seen_line}"))
    }

    pub async fn merge(&self, id: &str, only: Option<Vec<String>>) -> Result<String, String> {
        let (worker, parts) = self.finished_diff(id).await?;
        if worker.repo != self.worktrees.repo() {
            return Err(format!(
                "worker {id} was made from {}, but this server works in {}: start the planner there to merge it, or discard({id})",
                worker.repo.display(),
                self.worktrees.repo().display()
            ));
        }
        if parts.is_empty() {
            self.forget(id).await?;
            return Ok(format!("worker {id} has nothing left to merge; its worktree is removed"));
        }
        let chosen: Vec<&FileDiff> = match &only {
            None => parts.iter().collect(),
            Some(names) if names.is_empty() => return Err("`files` is empty: name the files to merge, or leave it out to merge all".into()),
            Some(names) => {
                let mut picked: Vec<&FileDiff> = Vec::new();
                for name in names {
                    let part = parts.iter().find(|p| &p.path == name).ok_or_else(|| {
                        format!("worker {id} has no file `{name}` to merge; files: {}", parts.iter().map(|p| p.path.as_str()).collect::<Vec<_>>().join(", "))
                    })?;
                    if !picked.contains(&part) {
                        picked.push(part);
                    }
                }
                picked
            }
        };
        let unseen: Vec<&str> = chosen.iter().map(|p| p.path.as_str()).filter(|p| !worker.shown.contains(*p)).collect();
        if !unseen.is_empty() {
            let seen: Vec<&str> = chosen.iter().map(|p| p.path.as_str()).filter(|p| worker.shown.contains(*p)).collect();
            let alternative = if seen.is_empty() {
                "none of the requested files has been shown yet".to_string()
            } else {
                format!("or merge again with `files` = [{}], the ones you have seen", seen.join(", "))
            };
            return Err(format!(
                "refusing to merge {} file(s) you have not seen in full: {}. Call show_diff for each of them first ({alternative}). Nothing was changed.",
                unseen.len(),
                unseen.join(", ")
            ));
        }
        let patch = join_diffs(chosen.iter().copied());
        let done: Vec<String> = chosen.iter().map(|p| p.path.clone()).collect();
        let turn = self.main_tree.lock().await;
        let applied = self.blocking(move |w| w.apply(&patch)).await;
        drop(turn);
        applied.map_err(|e| format!("{e:#}\nWorker {id}'s worktree is kept; discard({id}) it or merge by hand."))?;
        self.update(id, |w| w.merged.extend(done.iter().cloned())).await;
        let left: Vec<&str> = parts.iter().map(|p| p.path.as_str()).filter(|p| !done.iter().any(|d| d == p)).collect();
        if !left.is_empty() {
            return Ok(format!(
                "merged from worker {id}: {}. Changes are in your working tree. Not merged yet, still in the worker's worktree: {}. Merge or discard({id}) them later.",
                done.join(", "),
                left.join(", ")
            ));
        }
        let note = self.forget(id).await.err().map(|e| format!(" ({e})")).unwrap_or_default();
        Ok(format!("merged worker {id}: {}. Changes are in your working tree.{note}", done.join(", ")))
    }

    pub async fn discard(&self, id: &str) -> Result<String, String> {
        self.forget(id).await.map(|()| format!("discarded worker {id}"))
    }

    /// Take an auto-merged worker's changes back out of the main tree, if those files have
    /// not changed since in a way that stops the reverse patch.
    pub async fn undo(&self, id: &str) -> Result<String, String> {
        let applied = lock(&self.merged).get(id).cloned().ok_or_else(|| {
            format!("no auto-merged worker `{id}` to undo (undo works for workers auto-merged since this delegate server started)")
        })?;
        let _turn = self.main_tree.lock().await;
        let patch = applied.patch.clone();
        self.blocking(move |w| w.unapply(&patch)).await.map_err(|e| format!("could not undo worker {id}: {e:#}"))?;
        lock(&self.merged).remove(id);
        Ok(format!("undid worker {id}: {} are back to how they were before it merged", applied.files.join(", ")))
    }
}
