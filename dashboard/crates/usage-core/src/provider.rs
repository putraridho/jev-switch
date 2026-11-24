//! The dashboard's own list of providers (DESIGN 4.1).

use serde::{Deserialize, Serialize};

/// A provider Flashboard shows. Its own list: Antigravity is separate from Gemini for
/// billing. The mapping to the engine's provider id lives in `io::engine_bridge`.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub enum ProviderKey {
    /// Claude (Anthropic).
    #[serde(rename = "claude")]
    Claude,
    /// Codex / ChatGPT (OpenAI).
    #[serde(rename = "codex")]
    Codex,
    /// DeepSeek (API balance).
    #[serde(rename = "deepseek")]
    DeepSeek,
    /// Gemini CLI (Google).
    #[serde(rename = "gemini")]
    Gemini,
    /// Antigravity (Google).
    #[serde(rename = "antigravity")]
    Antigravity,
    /// Grok (xAI).
    #[serde(rename = "grok")]
    Grok,
}

impl ProviderKey {
    /// Every provider, in display and tie-break order.
    pub const ALL: [Self; 6] = [
        Self::Claude,
        Self::Codex,
        Self::DeepSeek,
        Self::Gemini,
        Self::Antigravity,
        Self::Grok,
    ];

    /// The one spelling used everywhere (storage, IPC, tests). Equal to the serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::DeepSeek => "deepseek",
            Self::Gemini => "gemini",
            Self::Antigravity => "antigravity",
            Self::Grok => "grok",
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
mod tests {
    use super::*;

    #[test]
    fn serde_name_equals_as_str_for_every_provider() {
        for p in ProviderKey::ALL {
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.as_str())
            );
            assert_eq!(
                serde_json::from_str::<ProviderKey>(&format!("\"{}\"", p.as_str())).unwrap(),
                p
            );
        }
    }

    #[test]
    fn deepseek_is_one_word() {
        assert_eq!(ProviderKey::DeepSeek.as_str(), "deepseek");
        assert!(serde_json::from_str::<ProviderKey>("\"deep_seek\"").is_err());
    }

    #[test]
    fn all_lists_each_provider_once_in_order() {
        let names: Vec<&str> = ProviderKey::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "claude",
                "codex",
                "deepseek",
                "gemini",
                "antigravity",
                "grok"
            ]
        );
        let mut sorted = ProviderKey::ALL;
        sorted.sort();
        assert_eq!(sorted, ProviderKey::ALL);
    }

    #[test]
    fn unknown_provider_is_refused() {
        for bad in ["\"openai\"", "\"Claude\"", "\"\"", "1", "null"] {
            assert!(serde_json::from_str::<ProviderKey>(bad).is_err(), "{bad}");
        }
    }
}
