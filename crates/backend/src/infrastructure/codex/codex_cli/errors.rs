//! Codex error wording.

use crate::{classify_common, ErrorKind};

pub(super) fn classify_error(message: &str) -> ErrorKind {
        if let Some(kind) = classify_common(message) {
            return kind;
        }
        let lower = message.to_lowercase();
        if lower.contains("not supported when using codex with a chatgpt account") {
            ErrorKind::NotEntitled
        } else if crate::has_token(&lower, "401") || lower.contains("missing bearer") {
            ErrorKind::Auth
        } else if ["not inside a git", "not a git repo", "requires a git repo"].iter().any(|p| lower.contains(p)) {
            ErrorKind::UnusableHere
        } else {
            ErrorKind::Other
        }
}
