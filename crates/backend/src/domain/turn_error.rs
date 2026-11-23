//! A failed turn.

use super::error_kind::ErrorKind;
use jev_shared::Usage;

/// A failed turn: `kind` for callers that react, `message` for people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnError {
    pub kind: ErrorKind,
    pub message: String,
    pub usage: Option<Usage>,
}

impl TurnError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), usage: None }
    }

    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TurnError {}
