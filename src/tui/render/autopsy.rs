//! `/why-failed`: after a failed turn, name the cause and the next action.

use crate::backend::ErrorKind;
use crate::tui::render::next_steps_for;

/// What went wrong, as far as the turn's record says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A provider failed with this kind of error.
    Provider { provider: String, kind: ErrorKind },
    /// A tool call reported an error.
    Tool { tool: String, message: String },
    /// The agent ran the tests and some failed.
    Tests { failed: usize },
    /// The turn was stopped by the user.
    Cancelled,
}

/// The cause in a phrase, and what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autopsy {
    pub cause: String,
    pub next: Vec<String>,
}

fn provider_cause(provider: &str, kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Quota { .. } => "quota: the usage limit was reached",
        ErrorKind::NotEntitled => "plan: the model is not part of the plan",
        ErrorKind::Auth => "login: the login or key is missing or expired",
        ErrorKind::Empty => "provider: it answered with nothing",
        ErrorKind::Stalled => "provider: it stopped responding",
        ErrorKind::UnusableHere => {
            if provider == "codex" || provider == "gemini" { "folder: the provider refuses to run in this folder" } else { "folder: the provider cannot run here" }
        }
        ErrorKind::Unsupported => "provider: this CLI is no longer supported by the service",
        ErrorKind::Other => "provider: it failed for a reason it did not say",
    }
}

/// Explain the latest failure.
pub fn autopsy(f: &Failure) -> Autopsy {
    match f {
        Failure::Provider { provider, kind } => Autopsy { cause: format!("{provider} {}", provider_cause(provider, kind)), next: next_steps_for(kind, provider) },
        Failure::Tool { tool, message } => Autopsy {
            cause: format!("tool: {tool} failed ({})", message.split_whitespace().collect::<Vec<_>>().join(" ")),
            next: vec!["Ask the agent to fix the cause shown above and try again".into(), "Check permissions with `/permissions` if it was denied".into()],
        },
        Failure::Tests { failed } => Autopsy {
            cause: format!("tests: {failed} test{} failed", if *failed == 1 { "" } else { "s" }),
            next: vec!["Send `fix the failing tests`, or look at the output above".into(), "Undo the last change with `/undo` if it made things worse".into()],
        },
        Failure::Cancelled => Autopsy { cause: "stopped: you cancelled the turn".into(), next: vec!["Send the message again, or `/undo` to drop its file changes".into()] },
    }
}

/// The `/why-failed` text: `Why it failed: <cause>` then `Next:` bullets; a turn that did
/// not fail says so.
pub fn why_failed_text(f: Option<&Failure>) -> String {
    let Some(f) = f else { return "The last turn did not fail.".into() };
    let a = autopsy(f);
    let mut out = format!("Why it failed: {}", a.cause);
    for step in &a.next {
        out.push_str(&format!("\n  Next: {step}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_kind_has_a_cause_and_a_next_step() {
        for kind in [
            ErrorKind::Quota { resets_at: Some("5pm".into()) },
            ErrorKind::NotEntitled,
            ErrorKind::Auth,
            ErrorKind::Empty,
            ErrorKind::Stalled,
            ErrorKind::UnusableHere,
            ErrorKind::Unsupported,
            ErrorKind::Other,
        ] {
            let a = autopsy(&Failure::Provider { provider: "codex".into(), kind: kind.clone() });
            assert!(a.cause.starts_with("codex "), "{kind:?}");
            assert!(!a.next.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn text_forms() {
        let q = Failure::Provider { provider: "claude".into(), kind: ErrorKind::Quota { resets_at: Some("5pm".into()) } };
        let text = why_failed_text(Some(&q));
        assert!(text.starts_with("Why it failed: claude quota: the usage limit was reached\n  Next: Wait for the limit to reset (5pm)"), "{text}");
        assert_eq!(why_failed_text(None), "The last turn did not fail.");
        assert_eq!(autopsy(&Failure::Tests { failed: 1 }).cause, "tests: 1 test failed");
        assert_eq!(autopsy(&Failure::Tests { failed: 3 }).cause, "tests: 3 tests failed");
        assert_eq!(autopsy(&Failure::Tool { tool: "Bash".into(), message: "exit\n 1".into() }).cause, "tool: Bash failed (exit 1)");
        assert!(autopsy(&Failure::Cancelled).cause.starts_with("stopped"));
    }
}
