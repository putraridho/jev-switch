//! Providers with no usage source the app can read yet (DESIGN 6.5): Gemini, Antigravity
//! and Grok. The source does no IO and always answers `Unsupported`.

use jev_usage_core::port::{
    Cost, FetchFuture, SourceDescriptor, SourceError, SourceId, Trigger, UsageSource,
};
use jev_usage_core::provider::ProviderKey;
use std::time::Duration;

/// The source of a provider that has none. Nothing ever signals it, so it is asked once at
/// start and when the window opens, never on a timer.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct PlaceholderSource {
    id: SourceId,
    provider: ProviderKey,
}

impl PlaceholderSource {
    /// The placeholder for Gemini, Antigravity or Grok. `None` for a provider that has a
    /// real source (Claude, Codex, DeepSeek): there is no placeholder id for it, so a
    /// placeholder can never stand in for a real adapter by mistake.
    pub fn new(provider: ProviderKey) -> Option<Self> {
        let id = match provider {
            ProviderKey::Gemini => SourceId::GeminiNone,
            ProviderKey::Antigravity => SourceId::AntigravityNone,
            ProviderKey::Grok => SourceId::GrokNone,
            ProviderKey::Claude | ProviderKey::Codex | ProviderKey::DeepSeek => return None,
        };
        Some(Self { id, provider })
    }
}

impl UsageSource for PlaceholderSource {
    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: self.id,
            provider: self.provider,
            trigger: Trigger::OnSignal,
            cost: Cost::Free,
            timeout: Duration::from_secs(1),
            fresh_for: Duration::from_mins(10),
            needs_login: false,
        }
    }

    fn fetch(&self) -> FetchFuture<'_> {
        Box::pin(std::future::ready(Err(SourceError::Unsupported)))
    }
}
