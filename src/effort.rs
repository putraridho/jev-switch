//! Reasoning effort: `/effort low|medium|high|auto`, and what each provider's CLI does with it.
//!
//! Claude Code (and DeepSeek through it) takes a thinking budget from `MAX_THINKING_TOKENS`;
//! Codex takes `model_reasoning_effort`. Gemini and Grok expose no such control through the
//! way jev-code drives them, so for them the setting is accepted and has no effect (`/effort`
//! says so). `auto` picks from the model tier that the router chose for the turn.

use std::fmt;
use std::str::FromStr;

use crate::shared::ProviderId;
use crate::catalog::{self, Tier};

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

/// What the person asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffortSetting {
    /// Nothing is passed: every CLI keeps its own default.
    #[default]
    Default,
    /// Each turn's effort follows the tier of the model the router picked.
    Auto,
    Fixed(Effort),
}

impl EffortSetting {
    /// The effort for a turn on `tier`: light models think little, top models think hard.
    /// `None` leaves the CLI's default alone.
    pub fn resolve(self, tier: Tier) -> Option<Effort> {
        match self {
            Self::Default => None,
            Self::Fixed(e) => Some(e),
            Self::Auto => Some(match tier {
                Tier::Light => Effort::Low,
                Tier::Everyday => Effort::Medium,
                Tier::Top => Effort::High,
            }),
        }
    }
}

impl fmt::Display for EffortSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::Auto => f.write_str("auto"),
            Self::Fixed(e) => f.write_str(e.name()),
        }
    }
}

impl FromStr for EffortSetting {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let word = s.trim().to_ascii_lowercase();
        match word.as_str() {
            "auto" => return Ok(Self::Auto),
            "default" | "off" => return Ok(Self::Default),
            _ => {}
        }
        Effort::ALL
            .into_iter()
            .find(|e| e.name() == word)
            .map(Self::Fixed)
            .ok_or_else(|| "usage: /effort [default|auto|low|medium|high]".to_string())
    }
}

/// Whether the provider's CLI is driven with an effort control.
pub fn supported(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::DeepSeek | ProviderId::Codex)
}

/// What the effort becomes for one provider, in words for `/effort`.
pub fn effect(provider: ProviderId, effort: Effort) -> String {
    match provider {
        ProviderId::Claude | ProviderId::DeepSeek => format!("thinking budget {} tokens", effort.thinking_tokens()),
        ProviderId::Codex => format!("reasoning effort {}", effort.name()),
        ProviderId::Gemini | ProviderId::Grok => "no effort control through this CLI".to_string(),
    }
}

/// The `/effort` screen: the setting, what it means per provider, and the context windows.
pub fn effort_text(setting: EffortSetting, ready: &[ProviderId]) -> String {
    let mut lines = vec![format!("  effort: {setting}")];
    for p in ready {
        let shown = match setting {
            EffortSetting::Default => "the CLI's own default".to_string(),
            EffortSetting::Fixed(e) => effect(*p, e),
            EffortSetting::Auto if supported(*p) => {
                let by_tier: Vec<String> = [Tier::Light, Tier::Everyday, Tier::Top]
                    .into_iter()
                    .filter_map(|t| EffortSetting::Auto.resolve(t).map(|e| format!("{} {}", tier_word(t), effect_short(*p, e))))
                    .collect();
                by_tier.join(", ")
            }
            EffortSetting::Auto => effect(*p, Effort::Medium),
        };
        let window = catalog::models().iter().filter(|m| m.provider == *p).map(|m| m.context_tokens).max();
        let window = window.map(|w| format!("  (context up to {})", crate::uifmt::human_count(w))).unwrap_or_default();
        lines.push(format!("    {:<9} {shown}{window}", p.as_str()));
    }
    lines.join("\n")
}

fn tier_word(t: Tier) -> &'static str {
    match t {
        Tier::Light => "light",
        Tier::Everyday => "everyday",
        Tier::Top => "top",
    }
}

fn effect_short(provider: ProviderId, effort: Effort) -> String {
    match provider {
        ProviderId::Claude | ProviderId::DeepSeek => format!("{}K", effort.thinking_tokens().div_ceil(1000)),
        _ => effort.name().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_spelling_and_rejects_the_rest() {
        assert_eq!("auto".parse(), Ok(EffortSetting::Auto));
        assert_eq!("HIGH".parse(), Ok(EffortSetting::Fixed(Effort::High)));
        assert_eq!("low".parse(), Ok(EffortSetting::Fixed(Effort::Low)));
        assert!("max".parse::<EffortSetting>().is_err());
    }

    #[test]
    fn auto_follows_the_tier_and_fixed_ignores_it() {
        assert_eq!(EffortSetting::Auto.resolve(Tier::Light), Some(Effort::Low));
        assert_eq!(EffortSetting::Auto.resolve(Tier::Top), Some(Effort::High));
        assert_eq!(EffortSetting::Fixed(Effort::Low).resolve(Tier::Top), Some(Effort::Low));
        assert_eq!(EffortSetting::Default.resolve(Tier::Top), None, "default passes nothing");
    }

    #[test]
    fn budgets_grow_with_effort() {
        let b: Vec<u32> = Effort::ALL.iter().map(|e| e.thinking_tokens()).collect();
        assert!(b.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_screen_says_what_each_provider_does() {
        let text = effort_text(EffortSetting::Fixed(Effort::High), &[ProviderId::Claude, ProviderId::Codex, ProviderId::Grok]);
        assert!(text.contains("effort: high") && text.contains("thinking budget 31999 tokens"), "{text}");
        assert!(text.contains("reasoning effort high") && text.contains("no effort control"), "{text}");
        let auto = effort_text(EffortSetting::Auto, &[ProviderId::Claude]);
        assert!(auto.contains("light 4K") && auto.contains("top 32K"), "{auto}");
    }
}
