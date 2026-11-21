//! A run of tool calls counted as one sentence.

use std::path::Path;

use super::tool::{display_tool, plural};
use crate::render::style::Style;
use crate::render::width::{clamp_width, truncate_to_width};

/// What a tool does, for counting a run of calls in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolKind {
    Edit,
    File,
    Search,
    Directory,
    Fetch,
    Command,
    Other,
}

pub(super) fn tool_kind(tool: &str) -> ToolKind {
    match display_tool(tool).to_ascii_lowercase().as_str() {
        "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" | "replace" | "write_file" | "edit_file" => ToolKind::Edit,
        "read" | "read_file" | "read_many_files" | "view" => ToolKind::File,
        "grep" | "glob" | "search" | "search_file_content" | "find" | "websearch" | "web_search" | "google_web_search" => ToolKind::Search,
        "ls" | "list_directory" | "list" => ToolKind::Directory,
        "webfetch" | "web_fetch" | "fetch" => ToolKind::Fetch,
        "bash" | "shell" | "exec" | "run_shell_command" | "exec_command" => ToolKind::Command,
        _ => ToolKind::Other,
    }
}

/// A run of tool calls between two things the model said, counted for one summary row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolGroup {
    /// Files an edit changed or created, by name, in the order first seen; `true` = created.
    files: Vec<(String, bool)>,
    added: usize,
    removed: usize,
    edit_calls: usize,
    reads: usize,
    searches: usize,
    directories: usize,
    fetches: usize,
    commands: usize,
    others: usize,
    failed: usize,
}

impl ToolGroup {
    /// One call, by its tool's name (`Read`, `Bash`, `mcp__srv__tool`).
    pub fn call(&mut self, tool: &str) {
        match tool_kind(tool) {
            ToolKind::Edit => self.edit_calls += 1,
            ToolKind::File => self.reads += 1,
            ToolKind::Search => self.searches += 1,
            ToolKind::Directory => self.directories += 1,
            ToolKind::Fetch => self.fetches += 1,
            ToolKind::Command => self.commands += 1,
            ToolKind::Other => self.others += 1,
        }
    }

    /// What an edit did to a file: lines taken out and put in. No lines taken out is a new file.
    pub fn edit(&mut self, path: &str, removed: usize, added: usize) {
        let name = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        if !self.files.iter().any(|(f, _)| *f == name) {
            self.files.push((name, removed == 0));
        }
        self.added += added;
        self.removed += removed;
    }

    pub fn failed(&mut self) {
        self.failed += 1;
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The run as phrases: `Edited a.rs`, `explored 1 directory, 3 files`, `ran 2 commands`.
    fn phrases(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.files.is_empty() || self.edit_calls > 0 {
            let verb = if !self.files.is_empty() && self.files.iter().all(|(_, created)| *created) { "Created" } else { "Edited" };
            let what = match self.files.len() {
                0 => plural(self.edit_calls, "file", "files"),
                1 | 2 => self.files.iter().map(|(f, _)| f.as_str()).collect::<Vec<_>>().join(", "),
                n => plural(n, "file", "files"),
            };
            out.push(format!("{verb} {what}"));
        }
        let explored: Vec<String> = [(self.directories, "directory", "directories"), (self.reads, "file", "files"), (self.searches, "search", "searches"), (self.fetches, "fetch", "fetches")]
            .into_iter()
            .filter(|(n, _, _)| *n > 0)
            .map(|(n, one, many)| plural(n, one, many))
            .collect();
        if !explored.is_empty() {
            out.push(format!("explored {}", explored.join(", ")));
        }
        if self.commands > 0 {
            out.push(format!("ran {}", plural(self.commands, "command", "commands")));
        }
        if self.others > 0 {
            out.push(format!("called {}", plural(self.others, "tool", "tools")));
        }
        out
    }
}

/// A run of tool calls as one row, the way it reads in a sentence:
/// `  ◇ Edited parser.rs, explored 3 files, 1 search, ran 2 commands +12 −4`. The leading verb
/// in bold, the rest dim, lines added green and taken out red, failures counted in red.
pub fn tool_group_row(group: &ToolGroup, style: Style, width: usize) -> String {
    let mut phrases = group.phrases();
    if phrases.is_empty() {
        return String::new();
    }
    let first = crate::render::text::capitalised(&phrases.remove(0));
    let (verb, rest) = first.split_once(' ').unwrap_or((&first, ""));
    let mut body = style.bold(verb);
    let tail: Vec<String> = std::iter::once(rest.to_string()).filter(|r| !r.is_empty()).chain(phrases).collect();
    if !tail.is_empty() {
        body.push_str(&style.dim(&format!(" {}", tail.join(", "))));
    }
    if group.added > 0 {
        body.push_str(&format!(" {}", style.green(&format!("+{}", group.added))));
    }
    if group.removed > 0 {
        body.push_str(&format!(" {}", style.red(&format!("−{}", group.removed))));
    }
    if group.failed > 0 {
        body.push_str(&format!(" {}", style.red(&format!("· {} failed", group.failed))));
    }
    truncate_to_width(&format!("  {} {body}", style.dim("◇")), clamp_width(width))
}
