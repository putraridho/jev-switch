//! Checking a prompt against the rules: which providers each rule refuses, and why.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::verdict::Verdict;
use super::{locality, Locality, Policy, Rule, Scope};
use crate::shared::domain::path_text::{glob_matches, mentioned_paths, relative};
use crate::shared::ProviderId;

impl Scope {
    fn covers(&self, provider: ProviderId) -> bool {
        match self {
            Self::AllCloud => locality(provider) == Locality::Cloud,
            Self::Providers(list) => list.contains(&provider),
        }
    }
}

impl Rule {
    fn glob(&self) -> &str {
        match self {
            Self::Block { glob, .. } | Self::Only { glob, .. } => glob,
        }
    }
}

impl Policy {
    /// What this prompt (and these attached files) may be sent to.
    pub fn check(&self, root: &Path, prompt: &str, attached: &[PathBuf], home: Option<&str>) -> Verdict {
        let mut paths = mentioned_paths(root, prompt, home);
        paths.extend(attached.iter().map(|p| relative(root, &p.to_string_lossy(), home)));
        let mut denials: BTreeMap<ProviderId, String> = BTreeMap::new();
        for rule in &self.rules {
            let Some(hit) = paths.iter().find(|p| glob_matches(rule.glob(), p)) else { continue };
            for provider in crate::shared::ALL_PROVIDERS {
                let denied = match rule {
                    Rule::Block { scope, .. } => scope.covers(provider),
                    Rule::Only { providers, .. } => !providers.contains(&provider),
                };
                if denied {
                    denials.entry(provider).or_insert_with(|| format!("{hit} is covered by `{}`", rule_text(rule)));
                }
            }
        }
        Verdict::new(self.local_only, denials)
    }
}

fn rule_text(rule: &Rule) -> String {
    match rule {
        Rule::Block { glob, scope } => format!("block {glob} from {}", scope_text(scope)),
        Rule::Only { glob, providers } => format!("only {} for {glob}", names(providers)),
    }
}

fn scope_text(scope: &Scope) -> String {
    match scope {
        Scope::AllCloud => "all".into(),
        Scope::Providers(p) => names(p),
    }
}

fn names(providers: &[ProviderId]) -> String {
    providers.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(",")
}
