use std::str::FromStr;

use super::Tier;

/// What Jev says a turn is about. The catalog maps each to an ordered model list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum TaskType {
    Security,
    HardDebug,
    Review,
    Feature,
    Frontend,
    Docs,
    Explain,
    CurrentInfo,
    SimpleEdit,
    Unclear,
}

impl TaskType {
    /// The tier this kind of work actually needs. A chain may start higher (Explain
    /// lists Grok 4.7 first because of who is good at it, not because explaining
    /// needs a top model), so "dropped a tier" is judged against this, not the chain.
    pub fn needed_tier(self) -> Tier {
        match self {
            Self::Security | Self::HardDebug | Self::Review => Tier::Top,
            Self::SimpleEdit => Tier::Light,
            Self::Feature | Self::Frontend | Self::Docs | Self::Explain | Self::CurrentInfo | Self::Unclear => Tier::Everyday,
        }
    }

    /// Every task type, so tests and config validation can loop over all of them.
    pub const ALL: [TaskType; 10] = [
        Self::Security,
        Self::HardDebug,
        Self::Review,
        Self::Feature,
        Self::Frontend,
        Self::Docs,
        Self::Explain,
        Self::CurrentInfo,
        Self::SimpleEdit,
        Self::Unclear,
    ];

    /// Stable snake_case name used by Jev, logs and the `JEV_CHAINS` file.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Security => "security",
            Self::HardDebug => "hard_debug",
            Self::Review => "review",
            Self::Feature => "feature",
            Self::Frontend => "frontend",
            Self::Docs => "docs",
            Self::Explain => "explain",
            Self::CurrentInfo => "current_info",
            Self::SimpleEdit => "simple_edit",
            Self::Unclear => "unclear",
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|t| t.as_str() == s).ok_or_else(|| format!("unknown task type `{s}`"))
    }
}
