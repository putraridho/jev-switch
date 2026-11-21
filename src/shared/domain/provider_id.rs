//! Provider identities and their stable names; this module performs no IO.

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

/// Every provider, in the order `/providers` lists them.
pub const ALL_PROVIDERS: [ProviderId; 5] =
    [ProviderId::Claude, ProviderId::DeepSeek, ProviderId::Codex, ProviderId::Gemini, ProviderId::Grok];

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
