//! How a provider's CLI is actually funded on this machine, as opposed to what the catalog
//! lists for the model. The catalog calls every Claude, Codex, Gemini and Grok model a
//! subscription model, which is only true while the CLI runs on its native login: an API key
//! is billed per token, and a custom endpoint is billed by someone the router cannot see.
//!
//! The facts come from the machine (adapters in `infrastructure/funding_probe`); the decision
//! here is pure. Only presence and kind are ever represented, never a secret.

use std::collections::BTreeMap;

use jev_backend::ProviderId;
use jev_catalog::domain::model_table as catalog;
use jev_catalog::domain::model_table::Billing;

pub mod codex;

/// What pays for one provider's CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Funding {
    /// The CLI's own saved login (a ChatGPT, Claude or Google account), the plan the catalog assumes.
    Login,
    /// An API key: billed per token.
    ApiKey,
    /// A configured non-default endpoint or backend: who bills, and how much, is not known.
    Custom,
    /// No login or key could be seen, so nothing can be claimed.
    Unknown,
}

/// What the machine shows about one CLI's credentials and endpoint, as presence only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Access {
    /// The CLI is set to talk to a non-default endpoint or provider.
    pub custom_endpoint: bool,
    /// An API key (or token) is configured and reaches the CLI.
    pub api_key: bool,
    /// A saved login is present.
    pub login: bool,
}

impl Funding {
    /// A custom endpoint decides first (it may charge anything), then a key (the CLIs prefer
    /// a key to a saved login, see the Grok gotcha in AGENTS.md), then a login.
    pub fn from_access(access: Access) -> Self {
        match access {
            Access { custom_endpoint: true, .. } => Self::Custom,
            Access { api_key: true, .. } => Self::ApiKey,
            Access { login: true, .. } => Self::Login,
            _ => Self::Unknown,
        }
    }

    /// Whether the flat plan the catalog assumes pays for this CLI.
    pub fn is_native(self) -> bool {
        self == Self::Login
    }

    /// A short honest label; a login is named, never called a subscription.
    pub fn label(self, provider: ProviderId) -> &'static str {
        match (self, provider) {
            (Self::Login, ProviderId::Codex) => "ChatGPT login",
            (Self::Login, ProviderId::Claude) => "Claude login",
            (Self::Login, ProviderId::Gemini) => "Google login",
            (Self::Login, ProviderId::Grok) => "Grok login",
            (Self::Login, ProviderId::DeepSeek) => "login",
            (Self::ApiKey, _) => "API key (per token)",
            (Self::Custom, _) => "custom endpoint (billing unknown)",
            (Self::Unknown, _) => "billing unknown",
        }
    }
}

/// The detected funding of each provider. A provider that was not inspected counts as its
/// native login, which keeps the catalog's own view (tests and tools that never probe).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fundings(BTreeMap<ProviderId, Funding>);

impl Fundings {
    pub fn with(mut self, provider: ProviderId, funding: Funding) -> Self {
        self.0.insert(provider, funding);
        self
    }

    /// What was detected; `None` when this provider was not inspected.
    pub fn get(&self, provider: ProviderId) -> Option<Funding> {
        self.0.get(&provider).copied()
    }

    pub fn of(&self, provider: ProviderId) -> Funding {
        self.get(provider).unwrap_or(Funding::Login)
    }

    /// Whether using the model costs money, or may, beyond the flat plan. A model the catalog
    /// does not know counts as paid, and so does a catalog subscription model whose CLI is
    /// not on its native login: the safe side of a cap. Unknown billing is controlled as paid
    /// without inventing a price.
    pub fn is_paid(&self, provider: ProviderId, model: &str) -> bool {
        catalog::find(provider, model).is_none_or(|m| m.billing != Billing::Subscription || !self.of(provider).is_native())
    }

    /// The `/providers` and doctor wording for one provider; `None` when it was not inspected.
    pub fn label(&self, provider: ProviderId) -> Option<&'static str> {
        self.get(provider).map(|f| f.label(provider))
    }
}

#[cfg(test)]
mod tests;
