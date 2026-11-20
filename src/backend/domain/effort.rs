//! How hard a model is asked to think, in the terms a provider's CLI takes.
//!
//! Claude Code (and DeepSeek through it) takes a thinking budget from `MAX_THINKING_TOKENS`;
//! Codex takes `model_reasoning_effort`. Which level a turn gets is decided in
//! [`crate::turn::domain::effort`]; this file only says what a level means to a CLI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub const ALL: [Effort; 3] = [Effort::Low, Effort::Medium, Effort::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// The thinking budget Claude Code is given, in tokens.
    pub fn thinking_tokens(self) -> u32 {
        match self {
            Self::Low => 4_000,
            Self::Medium => 10_000,
            Self::High => 31_999,
        }
    }
}
