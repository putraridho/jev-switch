//! A prompt as a CLI without a stdin hand-off receives it.

use std::path::{Path, PathBuf};

/// Longest prompt handed to a CLI as ONE command-line argument. Linux refuses a
/// single argument over 131072 bytes (`E2BIG`, "Argument list too long"), so stay well under.
pub const MAX_PROMPT_ARG_BYTES: usize = 100_000;

/// A prompt as a CLI that has no stdin hand-off has to receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptArg {
    /// Short enough to be the argument itself.
    Inline(String),
    /// Too long: the prompt is in `path` (private to the user) and `instruction` is a
    /// short argument that tells the model to read it.
    File { path: PathBuf, instruction: String },
}

impl PromptArg {
    /// The text for the CLI's prompt argument: the prompt, or the instruction to read the file.
    pub fn text(&self) -> &str {
        match self {
            Self::Inline(prompt) => prompt,
            Self::File { instruction, .. } => instruction,
        }
    }

    /// The file holding a long prompt, if there is one.
    pub fn file(&self) -> Option<&Path> {
        match self {
            Self::Inline(_) => None,
            Self::File { path, .. } => Some(path),
        }
    }
}
