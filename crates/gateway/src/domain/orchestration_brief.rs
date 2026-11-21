//! The jev-orchestrate brief a mod attaches to a prompt that triggered orchestration.
//! The gateway only reads it; it never edits the request.

/// Start of the context block the jev-orchestrate mod attaches to a prompt that
/// triggered orchestration.
pub const ORCH_MARKER: &str = "<jev-orchestrate>";

/// The prompt without the orchestration brief, for the decision log.
pub fn strip_orchestrate_block(prompt: &str) -> String {
    match prompt.find(ORCH_MARKER) {
        Some(i) => {
            let end = prompt[i..].find("</jev-orchestrate>").map(|e| i + e + "</jev-orchestrate>".len()).unwrap_or(prompt.len());
            format!("{}{}", &prompt[..i], &prompt[end..]).trim().to_string()
        }
        None => prompt.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brief_is_left_out_of_the_logged_prompt() {
        assert_eq!(strip_orchestrate_block("build X\n<jev-orchestrate>plan</jev-orchestrate>"), "build X");
        assert_eq!(strip_orchestrate_block("build X"), "build X");
    }
}
