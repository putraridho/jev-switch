//! The capability handshake: cheap checks that a provider's CLI still speaks what the adapter
//! relies on, and that the project's folder is one it will run in. Pure decisions over facts
//! the infrastructure gathers (`--version`, `--help`, an ACP `initialize` answer, folder trust).
//! Nothing here, and nothing the facts come from, sends a prompt or spends quota.

use jev_backend::ProviderId;

/// What one check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Passed(String),
    Failed(String),
    /// The check could not tell (it could not run, or there is nothing to look at).
    Unverified(String),
}

/// What the handshake found for one provider in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub protocol: Check,
    pub project: Check,
}

impl Handshake {
    pub fn unverified(why: &str) -> Self {
        Self { protocol: Check::Unverified(why.to_string()), project: Check::Unverified(why.to_string()) }
    }
}

/// The flags the adapter passes to the CLI in print mode, which `--help` must still list.
/// Empty for a CLI that is spoken to over ACP (see [`decide_acp`]).
pub fn required_flags(provider: ProviderId, bin: &str) -> &'static [&'static str] {
    match (provider, bin) {
        (ProviderId::Claude | ProviderId::DeepSeek, _) => &["--output-format", "--permission-mode"],
        (ProviderId::Codex, _) => &["exec"],
        (ProviderId::Gemini, "agy") => &[],
        (ProviderId::Gemini, _) => &["--output-format", "--approval-mode"],
        (ProviderId::Grok, _) => &[],
    }
}

/// Whether the CLI is spoken to over ACP, and with which arguments, so the handshake can ask
/// it to `initialize`.
pub fn acp_args(provider: ProviderId, bin: &str) -> Option<&'static [&'static str]> {
    match (provider, bin) {
        (ProviderId::Gemini, "agy") => Some(&["acp"]),
        (ProviderId::Grok, _) => Some(&["agent", "stdio"]),
        _ => None,
    }
}

/// `--version` and `--help` facts to a verdict. `version` is the first line of `--version`
/// when the command ran; `help` is `--help`'s text when it ran.
pub fn decide_flags(version: Option<&str>, help: Option<&str>, required: &[&str]) -> Check {
    let (Some(version), Some(help)) = (version, help) else {
        return Check::Unverified("the CLI did not answer --version or --help".to_string());
    };
    let version = if version.is_empty() { "version not printed" } else { version };
    let missing: Vec<&str> = required.iter().copied().filter(|flag| !help.contains(flag)).collect();
    // Only a real help text can say a flag is gone: anything else (a wrapper, an error page, a
    // fake) proves nothing, so it must not hold a provider.
    let lower = help.to_lowercase();
    if !missing.is_empty() && !lower.contains("usage:") && !lower.contains("options:") {
        return Check::Unverified("--help did not look like a help text".to_string());
    }
    if missing.is_empty() {
        Check::Passed(format!("{version}; supports {}", if required.is_empty() { "its print mode".to_string() } else { required.join(" ") }))
    } else {
        Check::Failed(format!("{version} no longer lists {}", missing.join(" ")))
    }
}

/// The first line an ACP agent answers to `initialize` (a JSON-RPC response) to a verdict.
pub fn decide_acp(answer: Option<&str>) -> Check {
    let Some(line) = answer.map(str::trim).filter(|l| !l.is_empty()) else {
        return Check::Unverified("the agent did not answer initialize in time".to_string());
    };
    if line.contains("\"result\"") && line.contains("protocolVersion") {
        Check::Passed("answered ACP initialize".to_string())
    } else if line.contains("\"error\"") {
        Check::Failed("refused ACP initialize".to_string())
    } else {
        Check::Unverified("answered something that is not ACP".to_string())
    }
}

/// What the machine shows about a folder's trust, for the CLIs that have such a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderFact {
    /// The CLI has no folder rule that can be read here.
    NotApplicable,
    /// A rule exists but nothing says what it does for this folder.
    Unknown,
    Trusted,
    Untrusted,
    InGitRepo,
    NotInGitRepo,
}

impl FolderFact {
    /// A word for the project key: the verdict changes when the folder's trust does.
    pub fn word(self) -> &'static str {
        match self {
            Self::NotApplicable => "",
            Self::Unknown => "unknown",
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::InGitRepo => "git",
            Self::NotInGitRepo => "no-git",
        }
    }
}

/// Whether `provider` will run in the folder.
pub fn decide_project(provider: ProviderId, fact: FolderFact) -> Check {
    match fact {
        FolderFact::Trusted => Check::Passed("folder is trusted".to_string()),
        FolderFact::InGitRepo => Check::Passed("folder is a git repository".to_string()),
        FolderFact::Untrusted => Check::Failed(format!("{provider} refuses an untrusted folder")),
        FolderFact::NotInGitRepo => Check::Failed(format!("{provider} needs a git repository")),
        FolderFact::Unknown => Check::Unverified("no trust decision for this folder is saved".to_string()),
        FolderFact::NotApplicable => Check::Unverified("no folder rule to check".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_must_still_be_listed() {
        let ok = decide_flags(Some("gemini 0.9"), Some("Usage: --output-format --approval-mode -m"), required_flags(ProviderId::Gemini, "gemini"));
        assert!(matches!(ok, Check::Passed(_)));
        let gone = decide_flags(Some("gemini 0.9"), Some("Usage: gemini [options]\nOptions:\n --output-format"), required_flags(ProviderId::Gemini, "gemini"));
        let not_help = decide_flags(Some("0.9"), Some(r#"{"type":"result","usage":{"input":1},"options":2}"#), required_flags(ProviderId::Gemini, "gemini"));
        assert!(matches!(not_help, Check::Unverified(_)), "output that is not a help text holds nothing");
        assert_eq!(gone, Check::Failed("gemini 0.9 no longer lists --approval-mode".into()));
        assert!(matches!(decide_flags(None, Some("x"), &["a"]), Check::Unverified(_)), "could not run: unknown, not fine");
    }

    #[test]
    fn acp_initialize_decides_for_agy_and_grok() {
        assert_eq!(acp_args(ProviderId::Gemini, "agy"), Some(&["acp"][..]));
        assert_eq!(acp_args(ProviderId::Grok, "grok"), Some(&["agent", "stdio"][..]));
        assert!(acp_args(ProviderId::Codex, "codex").is_none());
        assert!(matches!(decide_acp(Some(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#)), Check::Passed(_)));
        assert!(matches!(decide_acp(Some(r#"{"id":1,"error":{"code":-1}}"#)), Check::Failed(_)));
        assert!(matches!(decide_acp(Some("hello")), Check::Unverified(_)));
        assert!(matches!(decide_acp(None), Check::Unverified(_)));
    }

    #[test]
    fn folder_facts_decide_the_project_check() {
        assert!(matches!(decide_project(ProviderId::Gemini, FolderFact::Untrusted), Check::Failed(_)));
        assert!(matches!(decide_project(ProviderId::Codex, FolderFact::NotInGitRepo), Check::Failed(_)));
        assert!(matches!(decide_project(ProviderId::Gemini, FolderFact::Trusted), Check::Passed(_)));
        assert!(matches!(decide_project(ProviderId::Gemini, FolderFact::Unknown), Check::Unverified(_)));
        assert!(matches!(decide_project(ProviderId::Claude, FolderFact::NotApplicable), Check::Unverified(_)));
    }
}
