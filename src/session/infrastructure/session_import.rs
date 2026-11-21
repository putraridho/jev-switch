//! Bring a conversation from Claude Code or Codex into jev-code: read their history files
//! (JSON lines) into the turns a jev session holds, so `/resume` can continue it on any
//! provider. Typed readers for exactly the fields used; a line that does not fit is skipped,
//! so a format change loses detail, never the import.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

mod claude;
mod codex;
#[cfg(test)]
mod tests;
mod turns;

use claude::claude_turns;
use codex::{ codex_cwd, codex_rollouts, codex_turns };
use crate::session::infrastructure::session_file;
use crate::session::domain::saved_session::SavedSession;
use crate::session::domain::session::Sessions;
use crate::session::domain::turn_record::TurnRecord;

/// Where a history comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    ClaudeCode,
    Codex,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// One history file that belongs to the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub path: PathBuf,
    pub modified_ms: u64,
}

/// Most histories listed at once.
const MAX_FOUND: usize = 30;
/// Most Codex files looked into (their folder is shared by every project).
const MAX_CODEX_SCANNED: usize = 300;

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

/// Claude Code keeps a project's histories in `~/.claude/projects/<path with every
/// non-alphanumeric character turned into "-">/`.
pub fn claude_project_dir(home: &Path, project: &Path) -> PathBuf {
    let key = session_file::project_key(project);
    let encoded: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    home.join(".claude").join("projects").join(encoded)
}

/// The histories of `project` in `home`'s Claude Code and Codex folders, newest first.
pub fn discover(home: &Path, project: &Path) -> Vec<Found> {
    let mut found: Vec<Found> = jsonl_files(&claude_project_dir(home, project))
        .into_iter()
        .map(|path| Found { source: Source::ClaudeCode, modified_ms: modified_ms(&path), path })
        .collect();
    let key = session_file::project_key(project);
    let mut rollouts = codex_rollouts(&home.join(".codex").join("sessions"));
    rollouts.sort();
    rollouts.reverse();
    for path in rollouts.into_iter().take(MAX_CODEX_SCANNED) {
        if codex_cwd(&path).is_some_and(|cwd| session_file::project_key(Path::new(&cwd)) == key) {
            found.push(Found { source: Source::Codex, modified_ms: modified_ms(&path), path });
        }
    }
    found.sort_by_key(|b| std::cmp::Reverse(b.modified_ms));
    found.truncate(MAX_FOUND);
    found
}

/// What a history file holds, as jev turns.
pub fn load(found: &Found) -> Result<Vec<TurnRecord>, String> {
    let text = std::fs::read_to_string(&found.path).map_err(|e| e.to_string())?;
    let turns = match found.source {
        Source::ClaudeCode => claude_turns(&text),
        Source::Codex => codex_turns(&text),
    };
    if turns.is_empty() {
        return Err("no conversation found in that history".to_string());
    }
    Ok(turns)
}

/// The history as a saved session of `project`, last used when its file was last written.
pub fn to_saved(turns: Vec<TurnRecord>, project: &Path, saved_ms: u64) -> SavedSession {
    let mut sessions = Sessions::new();
    for turn in turns {
        sessions.push_unseen(turn);
    }
    SavedSession { project: session_file::project_key(project), saved_ms, sessions, current: None }
}
