//! What a turn's own tools did, read from the tool lines the backends report.

/// What a turn's own tools did, from the tool lines the backends report: the paths its
/// edit tools wrote, and whether anything ran that could write anywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnActivity {
    /// Paths named by edit/write tools, as reported (absolute or relative to the turn's folder).
    pub written: Vec<String>,
    /// A shell command, a sub-agent, an unknown tool, or a write tool without a usable
    /// path ran: it may have changed any file.
    pub opaque: bool,
    /// At least one tool line was reported. A turn that reported none may still have changed
    /// files (some CLIs report no tool events), so nothing can be told apart then.
    pub reported: bool,
}

/// Tools that write the one file their line names.
const WRITE_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "edit", "write_file", "replace", "edit_file", "write", "apply_patch", "create_file"];
/// Tools known not to change files.
const READ_TOOLS: &[&str] = &[
    "Read", "Grep", "Glob", "LS", "WebFetch", "WebSearch", "TodoWrite", "ExitPlanMode", "read_file", "list_directory", "glob", "grep",
    "search_file_content", "read_many_files", "web_fetch", "google_web_search", "web_search",
];
/// `tool_summary` cuts the telling field at this many characters; a path that long is cut.
const TOOL_FIELD_LIMIT: usize = 100;

impl TurnActivity {
    /// Classify tool lines such as `Edit /p/a.rs`, `edit /p/b.rs`, `Bash cargo test`, `Grep foo`.
    pub fn from_tools(tools: &[String]) -> Self {
        let mut a = Self::default();
        a.add_tools(tools);
        a
    }

    pub fn add_tools(&mut self, tools: &[String]) {
        for line in tools {
            self.reported = true;
            let (name, rest) = line.split_once(' ').map_or((line.as_str(), ""), |(n, r)| (n, r.trim()));
            if READ_TOOLS.contains(&name) {
                continue;
            }
            if WRITE_TOOLS.contains(&name) && !rest.is_empty() && rest.chars().count() < TOOL_FIELD_LIMIT {
                self.written.push(rest.to_string());
            } else {
                self.opaque = true;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::test_support::tools;

    #[test]
    fn tool_lines_are_classified_into_written_paths_and_opaque_runs() {
        let read_only = tools(&["Grep foo", "Read /r/a.rs", "Glob *.md"]);
        assert!(read_only.written.is_empty() && !read_only.opaque && read_only.reported);
        assert!(!TurnActivity::default().reported);
        let a = tools(&["Edit /r/a.rs", "edit /r/b c.rs", "MultiEdit src/x.rs"]);
        assert_eq!(a.written, ["/r/a.rs", "/r/b c.rs", "src/x.rs"]);
        assert!(!a.opaque);
        for opaque in ["Bash cargo test", "shell ls", "Task", "NotebookEdit", "Edit", "mcp__jev__delegate x"] {
            assert!(tools(&[opaque]).opaque, "{opaque}");
        }
        assert!(tools(&[&format!("Edit /{}", "x".repeat(150))]).opaque, "a cut-off path cannot be matched");
    }
}
