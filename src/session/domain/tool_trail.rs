//! What the tool summaries of a turn say was touched: the files written and the commands run.

/// Tool names (lowercase) that change files; their argument is the path.
const EDIT_TOOLS: &[&str] = &["edit", "write", "multiedit", "notebookedit", "apply_patch", "patch", "create"];
/// Tool names (lowercase) that run commands; their argument is the command line.
const COMMAND_TOOLS: &[&str] = &["bash", "shell", "command", "run"];

/// The argument of every summary whose tool name is in `names`, deduplicated in
/// first-seen order. Summaries are `Name args`, and backends differ in the case of
/// the name (`Edit` vs `edit`), so the name is matched case-insensitively.
fn args_of_tools(tools: &[String], names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for summary in tools {
        let summary = summary.trim();
        let (name, rest) = summary.split_once(char::is_whitespace).unwrap_or((summary, ""));
        let rest = rest.trim();
        if rest.is_empty() || !names.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        if !out.iter().any(|seen| seen == rest) {
            out.push(rest.to_string());
        }
    }
    out
}

/// Files the tool summaries say were written or edited.
pub fn files_touched(tools: &[String]) -> Vec<String> {
    args_of_tools(tools, EDIT_TOOLS)
}

/// Commands the tool summaries say were run.
pub fn commands_run(tools: &[String]) -> Vec<String> {
    args_of_tools(tools, COMMAND_TOOLS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn files_touched_and_commands_run_classify_by_tool_name() {
        let tools = strings(&[
            "Edit src/a.rs",
            "Write /abs/b.md",
            "MultiEdit f",
            "NotebookEdit n",
            "Bash cargo test",
            "shell cargo build",
            "edit src/c.rs",
            "apply_patch *** Begin Patch",
            "Edit src/a.rs",
            "Read src/z.rs",
            "Bash cargo test",
            "Bash",
        ]);
        assert_eq!(
            files_touched(&tools),
            strings(&["src/a.rs", "/abs/b.md", "f", "n", "src/c.rs", "*** Begin Patch"])
        );
        assert_eq!(commands_run(&tools), strings(&["cargo test", "cargo build"]));
    }
}
