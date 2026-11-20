//! Project memory: short notes the person wants every session of a project to start
//! with ("tests need docker up", "never touch the vendored folder"). Kept in the state
//! folder, not in the project, so nothing lands in the repository: `<state>/memory/<project
//! hash>.md`, one note per line. Complements AGENTS.md, which is for the whole team.

use std::path::{Path, PathBuf};

use crate::session::infrastructure::session_file::project_hash;

use crate::shared::infrastructure::private_file::write_atomic;

/// Longest note kept, in characters; a note is a reminder, not a document.
pub const MAX_NOTE_CHARS: usize = 300;

/// Most notes kept per project, so the memory cannot grow without bound into every prompt.
pub const MAX_NOTES: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    Empty,
    TooLong,
    Full,
    NoSuchNote(usize),
    Io(String),
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("write the note after `note`"),
            Self::TooLong => write!(f, "a note is at most {MAX_NOTE_CHARS} characters; keep long guidance in AGENTS.md"),
            Self::Full => write!(f, "{MAX_NOTES} notes already; remove one first (/project forget <number>)"),
            Self::NoSuchNote(n) => write!(f, "there is no note {n}; /project notes lists them"),
            Self::Io(e) => write!(f, "cannot save the notes: {e}"),
        }
    }
}

/// The notes of one project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    file: Option<PathBuf>,
    notes: Vec<String>,
}

impl Memory {
    /// The notes of `project_dir` kept under `state_dir` (none yet when the file is absent).
    pub fn load(state_dir: &Path, project_dir: &Path) -> Self {
        let file = state_dir.join("memory").join(format!("{}.md", project_hash(project_dir)));
        let notes = std::fs::read_to_string(&file).map(|t| parse(&t)).unwrap_or_default();
        Self { file: Some(file), notes }
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    pub fn add(&mut self, text: &str) -> Result<(), MemoryError> {
        let note = one_line(text);
        if note.is_empty() {
            return Err(MemoryError::Empty);
        }
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(MemoryError::TooLong);
        }
        if self.notes.len() >= MAX_NOTES {
            return Err(MemoryError::Full);
        }
        // The same note twice adds nothing.
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
        self.save()
    }

    /// Remove note `n` (counting from 1).
    pub fn forget(&mut self, n: usize) -> Result<String, MemoryError> {
        if n == 0 || n > self.notes.len() {
            return Err(MemoryError::NoSuchNote(n));
        }
        let gone = self.notes.remove(n - 1);
        self.save().map(|()| gone)
    }

    fn save(&self) -> Result<(), MemoryError> {
        let Some(file) = &self.file else { return Ok(()) };
        let text: String = self.notes.iter().map(|n| format!("- {n}\n")).collect();
        write_atomic(file, &text).map_err(|e| MemoryError::Io(e.to_string()))
    }

    /// What a session's first message carries; `None` when there are no notes.
    pub fn as_note(&self) -> Option<String> {
        (!self.notes.is_empty()).then(|| {
            let list: String = self.notes.iter().map(|n| format!("- {n}\n")).collect();
            format!("Notes the user keeps for this project (project memory):\n{}", list.trim_end())
        })
    }
}

/// A note on one line: line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.strip_prefix("- ")).map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::infrastructure::project_kind::tempdir::Dir;

    #[test]
    fn notes_survive_a_reload_and_are_per_project() {
        let state = Dir::new();
        let (a, b) = (Dir::new(), Dir::new());
        let mut mem = Memory::load(state.path(), a.path());
        mem.add("tests need   docker\nup").unwrap();
        mem.add("never edit vendor/").unwrap();
        mem.add("never edit vendor/").unwrap();
        assert_eq!(Memory::load(state.path(), a.path()).notes(), ["tests need docker up", "never edit vendor/"]);
        assert!(Memory::load(state.path(), b.path()).notes().is_empty());
    }

    #[test]
    fn forget_counts_from_one_and_rejects_a_missing_note() {
        let state = Dir::new();
        let p = Dir::new();
        let mut mem = Memory::load(state.path(), p.path());
        mem.add("one").unwrap();
        mem.add("two").unwrap();
        assert_eq!(mem.forget(1).unwrap(), "one");
        assert_eq!(Memory::load(state.path(), p.path()).notes(), ["two"]);
        assert_eq!(mem.forget(0), Err(MemoryError::NoSuchNote(0)));
        assert_eq!(mem.forget(5), Err(MemoryError::NoSuchNote(5)));
    }

    #[test]
    fn bad_notes_are_refused_and_the_note_for_the_model_lists_the_rest() {
        let mut mem = Memory::default();
        assert_eq!(mem.add("   "), Err(MemoryError::Empty));
        assert_eq!(mem.add(&"x".repeat(MAX_NOTE_CHARS + 1)), Err(MemoryError::TooLong));
        assert_eq!(mem.as_note(), None);
        for i in 0..MAX_NOTES {
            mem.add(&format!("n{i}")).unwrap();
        }
        assert_eq!(mem.add("one too many"), Err(MemoryError::Full));
        assert!(mem.as_note().unwrap().contains("- n39"));
    }
}
