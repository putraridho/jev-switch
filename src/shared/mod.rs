//! The types and infrastructure every context shares. Nothing here may depend on
//! any other module of this crate.

pub mod envfile;
pub mod log;

use std::str::FromStr;

/// Which service answers a turn. Several ids can share one CLI (DeepSeek is
/// `claude` pointed at another API), so this is not the same as the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    Claude,
    DeepSeek,
    Codex,
    Gemini,
    Grok,
}

impl ProviderId {
    /// Stable lowercase name used in logs, env vars and config.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::DeepSeek => "deepseek",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "deepseek" => Ok(Self::DeepSeek),
            "codex" => Ok(Self::Codex),
            "gemini" => Ok(Self::Gemini),
            "grok" => Ok(Self::Grok),
            other => Err(format!("unknown provider `{other}` (claude, deepseek, codex, gemini, grok)")),
        }
    }
}

/// Token counts for one turn. `input_tokens` is everything sent (fresh plus cached,
/// so providers compare on one scale) and `cached_input_tokens` is the cached share.
/// `context_tokens` is the size of the last request, when the CLI reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub context_tokens: Option<u64>,
}
