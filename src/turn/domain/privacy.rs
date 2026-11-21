//! The data-routing policy: what may leave this machine, and to whom.
//!
//! A policy file is plain lines (`#` starts a comment):
//!
//! ```text
//! local-only on                  # nothing goes to a cloud provider at all
//! redact on|off                  # blank out secrets before a prompt leaves (default on)
//! block secrets/** from all      # a prompt that names these files goes to no cloud provider
//! block *.pem from deepseek      # ... or not to one provider
//! only claude,codex for ops/**   # ... or only to these
//! ```
//!
//! The policy is checked on what the prompt NAMES (paths and `@file` mentions), because the
//! model reads the files itself through its own tools and this program cannot see that. So
//! this keeps a named path from being sent to the wrong vendor; it is not a sandbox, which
//! is what `/permissions` and `/mode` are for.
//!
//! Everything here is pure except `files` (reads the policy files) and `audit` (appends what
//! happened, never the prompt, never a secret, to a file). The child modules split it by
//! job: `parse` reads policy text, `settings` combines and toggles policies, `matching`
//! checks a prompt against the rules, `verdict` is the answer and its refusal text,
//! `outgoing` blanks secrets before a prompt leaves.

pub mod audit;
pub mod outgoing;
pub mod verdict;

mod files;
mod matching;
mod parse;
mod settings;
#[cfg(test)]
mod tests;

use std::fmt;

use crate::shared::ProviderId;
/// Where a provider's models run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    /// On this machine (an Ollama or llama.cpp endpoint).
    Local,
    /// Somebody else's servers.
    Cloud,
}

/// Every provider known today is a vendor's cloud; a local endpoint adapter says `Local`.
pub fn locality(_provider: ProviderId) -> Locality {
    Locality::Cloud
}

/// Whom a rule is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every cloud provider.
    AllCloud,
    Providers(Vec<ProviderId>),
}


#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    /// The paths matching `glob` must not go to `scope`.
    Block { glob: String, scope: Scope },
    /// The paths matching `glob` may only go to `providers`.
    Only { glob: String, providers: Vec<ProviderId> },
}


/// Whose file a policy came from. A repository can tighten the policy but not loosen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    User,
    Project,
}

/// A problem on one line of a policy file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

/// The rules in force.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    local_only: bool,
    /// Local-only came from a policy file or the environment, not from `/privacy`: the
    /// session cannot turn it off.
    local_only_locked: bool,
    redact: bool,
    rules: Vec<Rule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self { local_only: false, local_only_locked: false, redact: true, rules: Vec::new() }
    }
}
