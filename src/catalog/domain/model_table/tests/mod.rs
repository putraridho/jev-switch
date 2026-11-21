use super::*;
use crate::shared::ProviderId::{self, Claude, Codex, DeepSeek, Gemini, Grok};

mod catalog;
mod overrides;
mod pick;

const PROVIDERS: [ProviderId; 5] = [Claude, DeepSeek, Codex, Gemini, Grok];

fn providers_in(mask: u32) -> Vec<ProviderId> {
    PROVIDERS.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect()
}

fn open() -> Allowances {
    Allowances { credits: true, per_token: true }
}
