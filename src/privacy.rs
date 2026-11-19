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
//! Everything here is pure except [`AuditLog`], which appends what happened (never the
//! prompt, never a secret) to a file.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::backend::ProviderId;
use crate::secrets::{self, SecretKind};

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

impl Scope {
    fn covers(&self, provider: ProviderId) -> bool {
        match self {
            Self::AllCloud => locality(provider) == Locality::Cloud,
            Self::Providers(list) => list.contains(&provider),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rule {
    /// The paths matching `glob` must not go to `scope`.
    Block { glob: String, scope: Scope },
    /// The paths matching `glob` may only go to `providers`.
    Only { glob: String, providers: Vec<ProviderId> },
}

impl Rule {
    fn glob(&self) -> &str {
        match self {
            Self::Block { glob, .. } | Self::Only { glob, .. } => glob,
        }
    }
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

    /// `other`'s rules added to this one's. Local-only wins if either says so; redaction is
    /// off only when one of them says off (a project file cannot, see [`Source`]).
    pub fn merged(mut self, other: Policy) -> Self {
        self.local_only |= other.local_only;
        self.local_only_locked |= other.local_only_locked;
        self.redact &= other.redact;
        self.rules.extend(other.rules);
        self
    }

    /// The same policy with local-only forced on (`JEV_LOCAL_ONLY=1`, `/privacy local-only on`).
    pub fn with_local_only(mut self, on: bool) -> Self {
        self.local_only |= on;
        self.local_only_locked |= on;
        self
    }

    /// Turn local-only mode on or off for the session (unlike [`Self::with_local_only`],
    /// which can only add it). Turning it off is refused when a policy file or
    /// `JEV_LOCAL_ONLY` set it: those are the person's standing rule, not a session choice.
    pub fn set_local_only(&mut self, on: bool) -> Result<(), String> {
        if !on && self.local_only_locked {
            return Err("local-only is set by JEV_LOCAL_ONLY or a privacy.rules file; remove it there to turn it off".into());
        }
        self.local_only = on;
        Ok(())
    }

    pub fn local_only(&self) -> bool {
        self.local_only
    }

    pub fn redacts(&self) -> bool {
        self.redact
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// The policy files, user's first: `<state>/privacy.rules`, then `<project>/.jev/privacy.rules`.
    /// A missing file is an empty policy; an unreadable file forces local-only mode.
    pub fn load(state_dir: Option<&Path>, project: &Path) -> (Self, Vec<(PathBuf, PolicyError)>) {
        let mut policy = Self::default();
        let mut errors = Vec::new();
        let files = [
            (state_dir.map(|d| d.join(RULES_FILE)), Source::User),
            (Some(project.join(".jev").join(RULES_FILE)), Source::Project),
        ];
        for (path, source) in files {
            let Some(path) = path else { continue };
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                // A folder on the way that is a file means there is no rules file either.
                Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => continue,
                Err(error) => {
                    policy = policy.with_local_only(true);
                    errors.push((path, PolicyError { line: 0, message: format!("could not read privacy rules: {error}; local-only mode is on") }));
                    continue;
                }
            };
            let (loaded, errs) = Self::parse(&text, source);
            errors.extend(errs.into_iter().map(|e| (path.clone(), e)));
            policy = policy.merged(loaded);
        }
        (policy, errors)
    }

    /// What this prompt (and these attached files) may be sent to.
    pub fn check(&self, root: &Path, prompt: &str, attached: &[PathBuf]) -> Verdict {
        let mut paths = mentioned_paths(root, prompt);
        paths.extend(attached.iter().map(|p| relative(root, &p.to_string_lossy())));
        let mut denials: BTreeMap<ProviderId, String> = BTreeMap::new();
        for rule in &self.rules {
            let Some(hit) = paths.iter().find(|p| glob_matches(rule.glob(), p)) else { continue };
            for provider in crate::providers::ALL_PROVIDERS {
                let denied = match rule {
                    Rule::Block { scope, .. } => scope.covers(provider),
                    Rule::Only { providers, .. } => !providers.contains(&provider),
                };
                if denied {
                    denials.entry(provider).or_insert_with(|| format!("{hit} is covered by `{}`", rule_text(rule)));
                }
            }
        }
        Verdict { local_only: self.local_only, denials }
    }
}

const RULES_FILE: &str = "privacy.rules";

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

/// The outcome of [`Policy::check`] for one prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    local_only: bool,
    denials: BTreeMap<ProviderId, String>,
}

impl Verdict {
    /// Nothing is restricted.
    pub fn open() -> Self {
        Self { local_only: false, denials: BTreeMap::new() }
    }

    /// Whether the policy restricts this prompt at all (local-only, or a rule that fired).
    pub fn restricts_any(&self) -> bool {
        self.local_only || !self.denials.is_empty()
    }

    /// Why `provider` may not get this prompt, or `None` when it may.
    pub fn denial(&self, provider: ProviderId) -> Option<String> {
        if self.local_only && locality(provider) == Locality::Cloud {
            return Some("local-only mode is on".into());
        }
        self.denials.get(&provider).cloned()
    }

    /// Add `other`'s reason for refusing `provider` (if it has one and this verdict does not):
    /// for text only that provider will get, such as its catch-up.
    pub fn take_denial(&mut self, provider: ProviderId, other: &Verdict) {
        if let Some(why) = other.denials.get(&provider) {
            self.denials.entry(provider).or_insert_with(|| why.clone());
        }
    }

    pub fn permits(&self, provider: ProviderId) -> bool {
        self.denial(provider).is_none()
    }

    /// One line for the person when every provider is refused: the reasons, merged.
    pub fn refusal_line(&self) -> String {
        if self.local_only {
            return "local-only mode is on and no local provider is set up (see /privacy)".into();
        }
        let mut reasons: Vec<&String> = self.denials.values().collect();
        reasons.dedup();
        match reasons.first() {
            Some(r) => format!("privacy policy: {r}"),
            None => "privacy policy refuses every provider".into(),
        }
    }
}

// ------------------------------------------------------------------ paths and globs

/// The raw and unwrapped words of a prompt, relative to `root`.
pub fn mentioned_paths(root: &Path, prompt: &str) -> Vec<String> {
    let wrapper = |c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '?' | '!' | ':');
    prompt
        .split_whitespace()
        .flat_map(|raw| {
            let unwrapped = raw.trim_start_matches(|c| wrapper(c) || c == '@').trim_end_matches(|c| wrapper(c) || c == '.');
            let unwrapped = match unwrapped.rsplit_once(':') {
                Some((path, line)) if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) => path,
                _ => unwrapped,
            };
            [raw, unwrapped]
        })
        .filter(|w| !w.is_empty())
        .map(|w| relative(root, w))
        .collect()
}

/// `path` as the policy sees it: `~` expanded, `.` and `..` resolved (so `a/../secrets/x`
/// is `secrets/x`), and the project root taken off the front.
fn relative(root: &Path, path: &str) -> String {
    let home = std::env::var("HOME").ok().filter(|h| h.starts_with('/'));
    let expanded = match (path, &home) {
        ("~", Some(h)) => h.clone(),
        (p, Some(h)) if p.starts_with("~/") => format!("{}/{}", h.trim_end_matches('/'), &p[2..]),
        _ => path.to_string(),
    };
    let absolute = expanded.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in expanded.split('/') {
        match seg {
            "" | "." => {}
            ".." => match parts.last() {
                Some(last) if *last != ".." => {
                    parts.pop();
                }
                _ if absolute => {}
                _ => parts.push(".."),
            },
            s => parts.push(s),
        }
    }
    let normal = parts.join("/");
    let root = root.to_string_lossy();
    let root = root.trim_end_matches('/');
    if absolute && !root.is_empty() {
        let root_normal = root.trim_start_matches('/');
        if normal.eq_ignore_ascii_case(root_normal) {
            return String::new();
        }
        if let Some(prefix) = normal.get(..root_normal.len()) {
            if prefix.eq_ignore_ascii_case(root_normal) {
                if let Some(rest) = normal[root_normal.len()..].strip_prefix('/') {
                    return rest.to_string();
                }
            }
        }
    }
    normal
}

/// `*` and `?` inside a path segment, `**` as a whole segment for any number of segments.
/// A trailing `/` means everything below. A pattern with no `/` is tried on every segment
/// of the path, so `.env` and `*.pem` match at any depth. ASCII case is ignored.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = if pattern.ends_with('/') { format!("{pattern}**") } else { pattern.to_string() };
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pattern.contains('/') {
        let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        return segments_match(&pat, &path_segments);
    }
    path_segments.iter().any(|s| segment_matches(&pattern, s))
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments_match(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(p, tail)| segment_matches(first, p) && segments_match(rest, tail)),
    }
}

fn segment_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|n| go(rest, &t[n..])),
            Some(('?', rest)) => t.split_first().is_some_and(|(_, tail)| go(rest, tail)),
            Some((c, rest)) => t.split_first().is_some_and(|(x, tail)| x.eq_ignore_ascii_case(c) && go(rest, tail)),
        }
    }
    go(&p, &t)
}

// ------------------------------------------------------------------ before a prompt leaves

/// A prompt ready to send, and what was done to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub prompt: String,
    pub redacted: Vec<SecretKind>,
}

/// The prompt as it may leave: secrets blanked unless the policy turned redaction off.
pub fn prepare(policy: &Policy, prompt: &str) -> Outgoing {
    if !policy.redacts() {
        return Outgoing { prompt: prompt.to_string(), redacted: Vec::new() };
    }
    let r = secrets::redact(prompt);
    Outgoing { prompt: r.text, redacted: r.kinds }
}

// ------------------------------------------------------------------ audit log

/// What the audit log records about one thing that happened to a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    /// The prompt went to the provider (with secrets blanked, when `redacted` is not empty).
    Sent,
    /// The policy kept the prompt from the provider.
    Blocked,
}

/// One line of the audit log. It never holds the prompt or a secret: only sizes, secret
/// kinds and the policy's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub ts_ms: u64,
    pub action: AuditAction,
    pub provider: String,
    pub model: String,
    pub prompt_bytes: usize,
    pub redacted: Vec<String>,
    pub reason: Option<String>,
}

impl AuditRecord {
    pub fn sent(ts_ms: u64, provider: ProviderId, model: &str, out: &Outgoing) -> Self {
        Self {
            ts_ms,
            action: AuditAction::Sent,
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_bytes: out.prompt.len(),
            redacted: out.redacted.iter().map(|k| k.to_string()).collect(),
            reason: None,
        }
    }

    pub fn blocked(ts_ms: u64, provider: ProviderId, reason: String) -> Self {
        Self {
            ts_ms,
            action: AuditAction::Blocked,
            provider: provider.to_string(),
            model: String::new(),
            prompt_bytes: 0,
            redacted: Vec::new(),
            reason: Some(reason),
        }
    }
}

/// Append-only, one JSON line per record, readable only by its owner.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `<state>/privacy-audit.jsonl`.
    pub fn in_state_dir(dir: &Path) -> Self {
        Self::new(dir.join("privacy-audit.jsonl"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Never fails the turn: an audit line that cannot be written is dropped.
    pub fn record(&self, rec: &AuditRecord) {
        let Ok(mut line) = serde_json::to_string(rec) else { return };
        line.push('\n');
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        if let Ok(mut f) = opts.open(&self.path) {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// The last `n` records, oldest first; lines that do not parse are skipped.
    pub fn tail(&self, n: usize) -> Vec<AuditRecord> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let all: Vec<AuditRecord> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let skip = all.len().saturating_sub(n);
        all.into_iter().skip(skip).collect()
    }
}

impl fmt::Display for AuditRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.action {
            AuditAction::Sent if self.redacted.is_empty() => write!(f, "sent to {} {} ({} bytes)", self.provider, self.model, self.prompt_bytes),
            AuditAction::Sent => write!(f, "sent to {} {} with {} secret(s) blanked: {}", self.provider, self.model, self.redacted.len(), self.redacted.join(", ")),
            AuditAction::Blocked => write!(f, "kept from {}: {}", self.provider, self.reason.as_deref().unwrap_or("policy")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/work/app";

    fn policy(text: &str) -> Policy {
        let (p, errors) = Policy::parse(text, Source::User);
        assert!(errors.is_empty(), "{errors:?}");
        p
    }

    fn check(p: &Policy, prompt: &str) -> Verdict {
        p.check(Path::new(ROOT), prompt, &[])
    }

    #[test]
    fn globs() {
        let yes = [("secrets/**", "secrets/a/b.txt"), ("secrets/", "secrets/x"), ("*.pem", "deploy/keys/a.pem"), (".env", "app/.env"), ("src/*.rs", "src/a.rs"), ("**/prod/**", "a/b/prod/c"), ("a?c", "abc")];
        for (pat, path) in yes {
            assert!(glob_matches(pat, path), "{pat} should match {path}");
        }
        let no = [("secrets/**", "src/secrets.rs"), ("src/*.rs", "src/a/b.rs"), ("*.pem", "a.pem.txt"), ("secrets/**", "a/secrets/x"), (".env", ".environment")];
        for (pat, path) in no {
            assert!(!glob_matches(pat, path), "{pat} should not match {path}");
        }
    }

    #[test]
    fn punctuation_does_not_hide_paths() {
        let p = policy("block .env from all\nblock *.pem from all\nblock data? from all");
        for prompt in ["what's in .env?", ".env!", ".env,", ".env;", ".env:", "(.env)", "\".env\"", "'.env'", "`.env`", ".env.", "a.pem:", "@(.env)", ".env:42?"] {
            let verdict = check(&p, prompt);
            assert!(crate::providers::ALL_PROVIDERS.iter().all(|provider| !verdict.permits(*provider)), "{prompt}");
        }
        assert!(!check(&p, "data?").permits(ProviderId::Claude));
        let raw = policy("block data! from all");
        assert!(!check(&raw, "data!").permits(ProviderId::Claude));
        assert!(check(&p, "environment").permits(ProviderId::Claude));
    }

    #[test]
    fn globs_ignore_ascii_case_for_prompts_and_attachments() {
        let p = policy("block secrets/** from all\nblock *.pem from all");
        assert!(!check(&p, "Secrets/db.yml").permits(ProviderId::Claude));
        assert!(!check(&p, "/WORK/APP/Secrets/db.yml").permits(ProviderId::Claude));
        assert!(!check(&p, "deploy/KEY.PEM").permits(ProviderId::Claude));
        assert!(!p.check(Path::new(ROOT), "inspect", &[PathBuf::from("Secrets/db.yml")]).permits(ProviderId::Claude));
        assert!(glob_matches("SECRETS/**", "secrets/db.yml"));
        assert!(!glob_matches("secrets/**", "secrets-old/db.yml"));
    }

    #[test]
    fn a_blocked_path_in_the_prompt_refuses_those_providers_only() {
        let p = policy("block secrets/** from deepseek,grok\nblock *.pem from all");
        let v = check(&p, "look at @secrets/db.yml and fix it");
        assert!(!v.permits(ProviderId::DeepSeek) && !v.permits(ProviderId::Grok));
        assert!(v.permits(ProviderId::Claude) && v.permits(ProviderId::Codex));
        assert_eq!(v.denial(ProviderId::Grok).as_deref(), Some("secrets/db.yml is covered by `block secrets/** from deepseek,grok`"));
        let v = check(&p, "rotate \"deploy/a.pem\".");
        assert!(crate::providers::ALL_PROVIDERS.iter().all(|p| !v.permits(*p)));
        assert!(v.refusal_line().starts_with("privacy policy: deploy/a.pem"));
        // Absolute paths, ./ and :line suffixes still hit.
        assert!(!check(&p, "see /work/app/secrets/x.rs:12").permits(ProviderId::Grok));
        assert!(!check(&p, "see ./secrets/x.rs").permits(ProviderId::Grok));
        // A prompt that names nothing sensitive is open.
        assert_eq!(check(&p, "explain the parser"), Verdict::open());
    }

    #[test]
    fn only_rules_allow_a_list() {
        let p = policy("only claude,codex for ops/**");
        let v = check(&p, "edit ops/deploy.sh");
        assert!(v.permits(ProviderId::Claude) && v.permits(ProviderId::Codex));
        assert!(!v.permits(ProviderId::Gemini) && !v.permits(ProviderId::DeepSeek));
    }

    #[test]
    fn attached_files_count() {
        let p = policy("block *.png from all");
        let v = p.check(Path::new(ROOT), "what is this", &[PathBuf::from("/work/app/shots/a.png")]);
        assert!(!v.permits(ProviderId::Claude));
    }

    #[test]
    fn local_only_refuses_every_cloud_provider() {
        let p = policy("local-only on");
        let v = check(&p, "hello");
        assert!(crate::providers::ALL_PROVIDERS.iter().all(|p| !v.permits(*p)));
        assert!(v.refusal_line().contains("local-only"));
        assert!(Policy::default().with_local_only(true).local_only());
    }

    #[test]
    fn bad_lines_are_reported_and_the_rest_applies() {
        let (p, errors) = Policy::parse("redact maybe\nblock x from nobody\nfoo\nlocal-only on # now\n\n# c", Source::User);
        assert_eq!(errors.iter().map(|e| e.line).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(p.local_only());
        assert_eq!(p.rule_count(), 0);
    }

    #[test]
    fn unknown_providers_are_reported_without_dropping_known_ones() {
        let (p, errors) = Policy::parse("block secret from grok,unknown\nonly claude,unknown for ops/**\nblock other from unknown\nonly unknown for private/**", Source::User);
        assert_eq!(errors.iter().map(|error| error.line).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert!(errors.iter().all(|error| error.message.contains("unknown provider")));
        assert_eq!(p.rule_count(), 2);
        assert!(!check(&p, "secret").permits(ProviderId::Grok));
        assert!(check(&p, "secret").permits(ProviderId::Claude));
        assert!(check(&p, "ops/file").permits(ProviderId::Claude));
        assert!(!check(&p, "ops/file").permits(ProviderId::Grok));
    }

    #[test]
    fn a_project_file_cannot_loosen_the_policy() {
        let (p, errors) = Policy::parse("redact off", Source::Project);
        assert_eq!(errors.len(), 1);
        assert!(p.redacts());
        let (user, _) = Policy::parse("redact off", Source::User);
        assert!(!user.redacts());
        // Merging: the user's `off` is not undone by a file that says nothing.
        assert!(!user.clone().merged(Policy::default()).redacts());
        assert!(!Policy::default().merged(user).redacts());
    }

    #[test]
    fn prepare_blanks_secrets_unless_redaction_is_off() {
        let text = "use DB_PASSWORD=hunter2hunter2 here";
        let on = prepare(&Policy::default(), text);
        assert_eq!(on.prompt, "use DB_PASSWORD=[REDACTED:named-secret] here");
        assert_eq!(on.redacted, vec![SecretKind::NamedSecret]);
        let off = prepare(&policy("redact off"), text);
        assert_eq!(off.prompt, text);
        assert!(off.redacted.is_empty());
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-privacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn policy_files_load_from_state_and_project() {
        let state = temp("state");
        let project = temp("project");
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(state.join("privacy.rules"), "redact off\nblock a/** from grok\n").unwrap();
        std::fs::write(project.join(".jev/privacy.rules"), "local-only on\nredact off\n").unwrap();
        let (p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert_eq!(p.rule_count(), 1);
        assert_eq!(errors.len(), 1, "the project's `redact off` is refused");
        assert!(errors[0].0.ends_with(".jev/privacy.rules"));
        // Neither file: the default.
        let (none, errs) = Policy::load(None, &temp("empty"));
        assert_eq!(none, Policy::default());
        assert!(errs.is_empty());
    }

    #[test]
    fn unreadable_policy_files_force_locked_local_only_and_report_the_problem() {
        let state = temp("unreadable-state");
        let project = temp("unreadable-project");
        let file = state.join("privacy.rules");
        std::fs::write(&file, [0xff, 0xfe]).unwrap();
        let (mut p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert!(p.set_local_only(false).is_err());
        assert!(crate::providers::ALL_PROVIDERS.iter().all(|provider| !check(&p, "hello").permits(*provider)));
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, file);
        assert!(errors[0].1.to_string().contains("could not read privacy rules"));
        assert!(errors[0].1.to_string().contains("local-only"));

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let (p, errors) = Policy::load(Some(&state), &project);
        assert!(p.local_only());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, file);
    }

    #[test]
    fn the_audit_log_records_sizes_and_kinds_never_the_prompt() {
        let dir = temp("audit");
        let log = AuditLog::in_state_dir(&dir);
        let out = prepare(&Policy::default(), "x DB_PASSWORD=hunter2hunter2");
        log.record(&AuditRecord::sent(1, ProviderId::Claude, "opus", &out));
        log.record(&AuditRecord::blocked(2, ProviderId::Grok, "local-only mode is on".into()));
        log.record(&AuditRecord::sent(3, ProviderId::Codex, "gpt", &prepare(&Policy::default(), "plain")));
        let raw = std::fs::read_to_string(log.path()).unwrap();
        assert!(!raw.contains("hunter2") && !raw.contains("DB_PASSWORD"));
        let tail = log.tail(2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].action, AuditAction::Blocked);
        assert_eq!(tail[1].to_string(), "sent to codex gpt (5 bytes)");
        assert_eq!(log.tail(10)[0].to_string(), "sent to claude opus with 1 secret(s) blanked: named-secret");
        assert_eq!(tail[0].to_string(), "kept from grok: local-only mode is on");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(log.path()).unwrap().permissions()) & 0o777, 0o600);
        // A missing log is an empty tail.
        assert!(AuditLog::new(dir.join("none.jsonl")).tail(5).is_empty());
    }
}
