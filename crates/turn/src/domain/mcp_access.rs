//! Provider configuration relevant to an MCP-dependent turn.
use jev_shared::ProviderId;

#[derive(Debug, Clone)]
pub struct McpAccess {
    pub name: String,
    pub providers: Vec<ProviderId>,
}

impl McpAccess {
    pub fn mentioned(&self, prompt: &str) -> bool {
        let name = self.name.to_lowercase();
        !name.is_empty() && prompt.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .any(|word| word == name)
    }
}

/// Explicit access failures only; a generic tool error is not a reason to replay a turn.
pub fn access_failure(text: &str) -> bool {
    text.to_lowercase().split(['\n', '.', '!', '?']).any(|sentence| {
        sentence.contains("mcp") && [
            "failed to connect", "cannot connect", "can't connect", "could not connect",
            "unable to connect", "cannot access", "can't access", "don't have access",
            "do not have access", "not connected", "no mcp servers", "no mcp tools",
            "mcp server unavailable", "mcp server is unavailable",
        ].iter().any(|phrase| sentence.contains(phrase))
    })
}

/// A refusal to perform the turn, rather than an explanation of a bug that was fixed.
pub fn access_refusal(text: &str) -> bool {
    let text = text.trim().to_lowercase();
    let starts_with_refusal = [
        "i cannot", "i can't", "i could not", "i couldn't", "i am unable", "i'm unable",
        "i don't have", "i do not have", "unable to", "cannot ", "can't ", "failed to",
        "error:", "mcp server", "no mcp",
    ].iter().any(|prefix| text.starts_with(prefix));
    starts_with_refusal && access_failure(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_a_server_name_as_a_whole_word() {
        let access = McpAccess { name: "linear".into(), providers: vec![ProviderId::Claude] };
        assert!(access.mentioned("Use Linear to find my issues"));
        assert!(!access.mentioned("Implement nonlinear regression"));
    }
    #[test]
    fn a_successful_fix_description_is_not_a_refusal() {
        assert!(!access_refusal("Fixed the MCP bug: failed to connect was incorrectly treated as success."));
        assert!(!access_failure("MCP tools worked. Cannot connect to the database."));
        assert!(access_refusal("I cannot access the Linear MCP server."));
    }
    #[test]
    fn distinguishes_access_failure_from_tool_errors() {
        assert!(access_failure("I cannot access the Linear MCP server."));
        assert!(!access_failure("MCP tool failed: invalid issue id"));
        assert!(!access_failure("Cannot connect to the test database"));
    }
}
