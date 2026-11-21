//! What one tool call asks and how it was answered, and the one-line texts for the answers.

use jev_shared::domain::text::one_line;


/// The hint when a tool call was denied because nobody could be asked (`-p`, a pipe).
pub fn denied_without_asking(tool: &str) -> String {
    format!(
        "\u{2717} denied {} \u{2014} nobody to ask in this run; allow it with CODE_ALLOWED_TOOLS, or in an interactive session answer the question or use /permissions",
        one_line(tool)
    )
}

/// The line shown when auto mode allowed a call without asking.
pub fn auto_allowed(summary: &str) -> String {
    format!("\u{2713} auto-allowed {}", one_line(summary))
}

/// The line shown when `CODE_ALLOWED_TOOLS` allowed a call without asking.
pub fn allowed_by_setting(summary: &str) -> String {
    format!("\u{2713} allowed (CODE_ALLOWED_TOOLS) {}", one_line(summary))
}

/// The hint for a call every rule of which was saved before ("always"): allowed unasked.
pub fn remembered_allowed(summary: &str) -> String {
    format!("\u{2713} allowed (remembered) {}", one_line(summary))
}

/// A tool call the CLI wants approval for.
#[derive(Clone, PartialEq)]
pub struct PermissionRequest {
    /// The CLI's id for the question; the answer must carry it back.
    pub id: String,
    pub tool: String,
    /// One line for the person: the command, or the path (and size) for a file write.
    pub summary: String,
    /// The tool's input, read field by field by the rules and the review; its raw form is
    /// echoed back to the CLI. It can hold secrets: it is never logged.
    pub input: crate::domain::tool_input::ToolInput,
    /// The path outside the allowed folders that made the CLI ask, when it said.
    pub blocked_path: Option<String>,
}

impl std::fmt::Debug for PermissionRequest {
    /// Leaves out the summary and the input: both can hold a secret, and `{:?}` ends up in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionRequest").field("id", &self.id).field("tool", &self.tool).finish_non_exhaustive()
    }
}

/// What happens to a [`PermissionRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it this once.
    Allow,
    /// Run it, and the approver has remembered a rule so the CLI does not ask again.
    AllowAlways,
    /// Do not run it; the model is told `reason`.
    Deny { reason: String },
}
