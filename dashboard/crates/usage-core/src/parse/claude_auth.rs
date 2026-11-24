//! The whole output of `claude auth status` (DESIGN 8.3, 8.6). T01 writes the body.

use crate::label::Label;

/// What the login probe keeps of `claude auth status`: nothing that identifies the account.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ClaudeLogin {
    /// Whether the CLI is signed in.
    pub logged_in: bool,
    /// The plan name, when the output carries one.
    pub plan: Option<Label>,
}

/// The three-field allow-list over the JSON form, else the text rule; `None` when neither
/// says anything.
pub fn parse(output: &[u8]) -> Option<ClaudeLogin> {
    let _ = output;
    None
}
