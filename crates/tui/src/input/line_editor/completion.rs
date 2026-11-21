//! Tab completion: command words from the command table and `@path` mentions.

use std::path::Path;

use rustyline::completion::{Completer, Pair};
use rustyline::Context;

use super::JevHelper;
use crate::render as uifmt;
use crate::render::CompleteCtx;

/// Most `@path` candidates Tab lists at once.
const MAX_PATH_CANDIDATES: usize = 200;

/// Candidates for an `@path` word (given without the `@`): the entries of its folder that
/// start with what was typed, folders with a `/`; and, for a word with no `/` yet, the
/// tracked files anywhere whose name starts with it (`@main` finds `src/main.rs`). Hidden
/// entries only when typed with a dot; `.git` never. Spaces are escaped, as a pasted
/// path would be.
pub fn path_candidates(word: &str, project: &Path, tracked: &[String]) -> Vec<Pair> {
    let (folder, prefix) = word.rsplit_once('/').map_or(("", word), |(d, p)| (d, p));
    let lower = prefix.to_lowercase();
    let mut found: Vec<(String, bool)> = Vec::new();
    let base = if folder.is_empty() { project.to_path_buf() } else { project.join(folder) };
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let hidden_ok = !name.starts_with('.') || prefix.starts_with('.');
            if name == ".git" || !hidden_ok || !name.to_lowercase().starts_with(&lower) {
                continue;
            }
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            let path = if folder.is_empty() { name } else { format!("{folder}/{name}") };
            found.push((path, is_dir));
        }
    }
    found.sort();
    if !word.contains('/') && !word.is_empty() {
        let deeper = tracked.iter().filter(|t| t.contains('/') && t.rsplit('/').next().is_some_and(|n| n.to_lowercase().starts_with(&lower)));
        found.extend(deeper.map(|t| (t.clone(), false)));
    }
    found.dedup();
    found
        .into_iter()
        .take(MAX_PATH_CANDIDATES)
        .map(|(path, is_dir)| {
            let shown = if is_dir { format!("{path}/") } else { path };
            Pair { display: shown.clone(), replacement: format!("@{}", shown.replace(' ', "\\ ")) }
        })
        .collect()
}

/// The files git tracks in `dir` (empty outside a repository).
pub fn tracked_files(dir: &Path) -> Vec<String> {
    let out = std::process::Command::new("git").args(jev_shared::infrastructure::git_guard::no_repo_programs(dir)).args(["ls-files", "-z"]).current_dir(dir).stderr(std::process::Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// The completions of the word before the cursor: where that word starts, and the
/// candidates. Pure, so Tab behaviour is tested without a terminal.
pub fn completions_for(line: &str, pos: usize, ctx: &CompleteCtx) -> (usize, Vec<Pair>) {
    let upto = line.get(..pos).unwrap_or(line);
    let candidates = uifmt::complete(upto, ctx)
        .into_iter()
        .map(|c| Pair { display: c.display, replacement: c.replacement })
        .collect();
    (uifmt::completion_start(upto), candidates)
}

impl Completer for JevHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let upto = line.get(..pos).unwrap_or(line);
        let start = uifmt::completion_start(upto);
        if let (Some(word), Some(project)) = (upto[start..].strip_prefix('@'), &self.project) {
            return Ok((start, path_candidates(word, project, &tracked_files(project))));
        }
        Ok(completions_for(line, pos, &self.ctx))
    }
}
