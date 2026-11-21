//! The query a person types: free words plus `key:value` filters.

use std::str::FromStr;

use super::*;
use crate::backend::ProviderId;

/// Which sessions a listing is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// Not archived.
    #[default]
    Active,
    Archived,
    All,
}

/// What a person asked the library for: free words plus `key:value` filters, e.g.
/// `parser provider:codex tag:bug file:lexer.rs since:7d project:jev`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub provider: Option<ProviderId>,
    pub tag: Option<String>,
    pub file: Option<String>,
    pub project: Option<String>,
    /// Only sessions saved within this many milliseconds.
    pub since_ms: Option<u64>,
    pub pinned_only: bool,
    pub scope: Scope,
}

impl Query {
    /// `Err` says which filter was not understood.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut q = Self::default();
        for token in text.split_whitespace() {
            match token.split_once(':') {
                Some(("provider", v)) => q.provider = Some(ProviderId::from_str(v).map_err(|_| format!("unknown provider \"{v}\""))?),
                Some(("tag", v)) => q.tag = Some(crate::session::infrastructure::session_meta::clean_tag(v).ok_or_else(|| format!("\"{v}\" is not a tag"))?),
                Some(("file", v)) if !v.is_empty() => q.file = Some(v.to_lowercase()),
                Some(("project", v)) if !v.is_empty() => q.project = Some(v.to_lowercase()),
                Some(("since", v)) => q.since_ms = Some(parse_days(v).ok_or_else(|| format!("since:{v} should look like since:7d or since:2w"))?),
                _ if token == "pinned" => q.pinned_only = true,
                _ if token == "archived" => q.scope = Scope::Archived,
                _ if token == "all" => q.scope = Scope::All,
                _ => q.words.push(token.to_lowercase()),
            }
        }
        Ok(q)
    }

    pub(super) fn admits(&self, e: &LibraryEntry, now_ms: u64) -> bool {
        let in_scope = match self.scope {
            Scope::Active => !e.meta.archived,
            Scope::Archived => e.meta.archived,
            Scope::All => true,
        };
        in_scope
            && (!self.pinned_only || e.meta.pinned)
            && self.provider.is_none_or(|p| e.used(p))
            && self.tag.as_ref().is_none_or(|t| e.meta.tags.contains(t))
            && self.file.as_ref().is_none_or(|f| e.mentions_file(f))
            && self.project.as_ref().is_none_or(|p| e.saved.project.to_lowercase().contains(p))
            && self.since_ms.is_none_or(|ms| now_ms.saturating_sub(e.saved.saved_ms) <= ms)
    }
}

/// `7d` or `2w` in milliseconds.
fn parse_days(text: &str) -> Option<u64> {
    // Split before the last character, not the last byte: `7日` must be refused, not panic.
    let (at, unit) = text.char_indices().last()?;
    let n: u64 = text[..at].parse().ok()?;
    match unit {
        'd' => Some(n.saturating_mul(DAY_MS)),
        'w' => Some(n.saturating_mul(7 * DAY_MS)),
        _ => None,
    }
}
