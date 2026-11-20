//! Why a turn failed.

/// Why a turn failed, as far as the caller needs to react differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Usage limit or rate limit; `resets_at` is the CLI's own wording, if any.
    Quota { resets_at: Option<String> },
    /// The model or feature is not part of the user's plan.
    NotEntitled,
    /// Not logged in, or the login or key expired.
    Auth,
    /// The CLI finished with an empty answer.
    Empty,
    /// No output for the whole idle timeout.
    Stalled,
    /// The CLI refuses to run in this directory (Codex outside a git repo, Gemini CLI
    /// in an untrusted workspace). Another provider would work, so a turn fails over.
    UnusableHere,
    Other,
}
