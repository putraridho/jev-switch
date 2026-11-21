//! Reading policy text: one rule per line, problems reported and valid rules still applied.

use super::{Policy, PolicyError, Rule, Scope, Source};

impl Policy {
    /// Read one policy file. Problems are reported and valid rules still apply.
    pub fn parse(text: &str, source: Source) -> (Self, Vec<PolicyError>) {
        let mut policy = Self::default();
        let mut errors = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Err(message) = policy.apply_line(line, source) {
                errors.push(PolicyError { line: i + 1, message });
            }
        }
        (policy, errors)
    }

    fn apply_line(&mut self, line: &str, source: Source) -> Result<(), String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["local-only", v] => {
                self.local_only = on_off(v)?;
                self.local_only_locked |= self.local_only;
            }
            ["redact", v] => {
                let on = on_off(v)?;
                if !on && source == Source::Project {
                    return Err("a project file cannot turn redaction off; only your own policy file can".into());
                }
                self.redact = on;
            }
            ["block", glob, "from", who] => {
                let (scope, warning) = parse_scope(who)?;
                self.rules.push(Rule::Block { glob: (*glob).into(), scope });
                if let Some(warning) = warning { return Err(warning); }
            }
            ["only", who, "for", glob] => {
                let (scope, warning) = parse_scope(who)?;
                match scope {
                    Scope::Providers(providers) => self.rules.push(Rule::Only { glob: (*glob).into(), providers }),
                    Scope::AllCloud => return Err("`only` needs provider names, like `only claude,codex for ops/**`".into()),
                }
                if let Some(warning) = warning { return Err(warning); }
            }
            _ => return Err(format!("not a rule: `{line}` (see /privacy for the syntax)")),
        }
        Ok(())
    }
}

fn on_off(word: &str) -> Result<bool, String> {
    match word {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("expected `on` or `off`, got `{other}`")),
    }
}

fn parse_scope(word: &str) -> Result<(Scope, Option<String>), String> {
    if matches!(word, "all" | "cloud") {
        return Ok((Scope::AllCloud, None));
    }
    let mut providers = Vec::new();
    let mut errors = Vec::new();
    for name in word.split(',') {
        match name.parse() {
            Ok(provider) => providers.push(provider),
            Err(error) => errors.push(error),
        }
    }
    let warning = if errors.is_empty() { None } else { Some(errors.join("; ")) };
    if providers.is_empty() {
        return Err(warning.unwrap_or_else(|| "no providers specified".into()));
    }
    Ok((Scope::Providers(providers), warning))
}
