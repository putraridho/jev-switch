//! What the person approves with one key: the whole of a tool call, as safe text.

use std::path::Path;

use serde_json::Value;
use crate::approval::domain::tool_input::{fields, str_field, value_text};
use crate::approval::domain::visible_text::visible;

/// What one answer approves, as plain text that is safe to print: control and escape
/// characters, invisible characters and odd whitespace are replaced by visible markers (see
/// [`visible`]), so nothing in the input can move the cursor, clear the screen or hide text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Review {
    /// Facts about the call that are not part of the input text (options of a Bash call):
    /// printed apart from it, so the input cannot imitate them.
    pub notes: Vec<String>,
    /// The input itself, one entry per line. Never cut: the layout wraps it, and pages it
    /// when it is long.
    pub lines: Vec<String>,
}

/// A short label for a request, for places that have one line (logs, tests). The approval
/// question is built from [`review`], which is complete; this is not what the person reads.
pub fn describe(tool: &str, input: &Value) -> String {
    let lines = review(tool, input, None).lines;
    match tool {
        "Write" => lines.into_iter().next().unwrap_or_default(),
        _ => lines.join("\n"),
    }
}

/// Everything the person approves with one key, for `tool` and its `input`: the command (all
/// of it), the path and every line of what is written, the telling field of a known tool, and
/// every field of a tool this module does not know.
///
/// What is shown is what will run or be written, byte for byte (see [`visible`] for the one
/// change, control characters). The single exception is a field that holds a PATH
/// (`file_path`, `path`, `notebook_path`): with a `project`, a path inside it is shown relative
/// to it, which names the same place because the tool runs from there. A command, the content
/// of a write, an edit, a pattern or a prompt is never rewritten.
pub fn review(tool: &str, input: &Value, project: Option<&Path>) -> Review {
    match tool {
        "Bash" => bash_review(input),
        "Write" => Review { lines: write_lines(input, project), ..Review::default() },
        "Edit" => Review { lines: edit_lines(input, project), ..Review::default() },
        "MultiEdit" => Review { lines: multi_edit_lines(input, project), ..Review::default() },
        "NotebookEdit" => Review { lines: notebook_lines(input, project), ..Review::default() },
        _ => Review { lines: telling_lines(tool, input, project).unwrap_or_else(|| all_fields(input)), ..Review::default() },
    }
}

/// The input fields that hold a path, the only ones whose text [`review`] may shorten.
const PATH_FIELDS: &[&str] = &["file_path", "path", "notebook_path"];

/// `path` relative to `project` when it is inside it (`.` for the project itself), else as it
/// is. Compared by path components, so `/p2/x` is not inside `/p`.
fn shorten_path(path: &str, project: Option<&Path>) -> String {
    match project.and_then(|p| Path::new(path).strip_prefix(p).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => ".".to_string(),
        Some(rest) => rest.to_string_lossy().into_owned(),
        None => path.to_string(),
    }
}

/// The text of one input field: a path field shortened, anything else exactly as it is.
fn field_text(key: &str, v: &Value, project: Option<&Path>) -> String {
    let text = value_text(v);
    if PATH_FIELDS.contains(&key) && v.is_string() {
        shorten_path(&text, project)
    } else {
        text
    }
}

/// The lines of `text`, each made [`visible`]. One line break at the very end is not a line.
fn text_lines(text: &str) -> Vec<String> {
    let mut parts: Vec<&str> = text.split('\n').collect();
    if parts.len() > 1 && parts.last() == Some(&"") {
        parts.pop();
    }
    parts.into_iter().map(visible).collect()
}

/// `key=value`, a multi-line value continued on indented lines.
fn key_value_lines(key: &str, text: &str) -> Vec<String> {
    let mut lines = text_lines(text).into_iter();
    let first = lines.next().unwrap_or_default();
    std::iter::once(format!("{}={first}", visible(key))).chain(lines.map(|l| format!("  {l}"))).collect()
}

/// Every field of the input: how an unknown tool (an MCP tool) is shown, so none of what
/// its call carries is left out.
fn all_fields(input: &Value) -> Vec<String> {
    match input {
        Value::Object(map) if !map.is_empty() => map.iter().flat_map(|(k, v)| key_value_lines(k, &value_text(v))).collect(),
        Value::Null | Value::Object(_) => vec!["(no input)".to_string()],
        other => text_lines(&other.to_string()),
    }
}

/// The fields that say what a known tool does, or `None` for a tool this does not know (or
/// one that carries none of them).
fn telling_lines(tool: &str, input: &Value, project: Option<&Path>) -> Option<Vec<String>> {
    let keys: &[&str] = match tool {
        "Read" => &["file_path"],
        "WebSearch" => &["query"],
        "WebFetch" => &["url"],
        "LS" => &["path"],
        "Glob" => &["pattern", "path"],
        "Grep" => &["pattern", "path", "glob"],
        "Task" | "Agent" => return Some(task_lines(input)),
        _ => return None,
    };
    let present: Vec<(&str, &Value)> = keys.iter().filter_map(|k| input.get(*k).map(|v| (*k, v))).collect();
    match present.as_slice() {
        [] => None,
        [(k, v)] if keys.len() == 1 => Some(text_lines(&field_text(k, v, project))),
        _ => Some(present.iter().flat_map(|(k, v)| key_value_lines(k, &field_text(k, v, project))).collect()),
    }
}

/// A sub-agent call: what it is for, which agent, and the PROMPT it will be given (the
/// instructions are what the approval is really about, and a long one goes through the pager
/// like any long input), then every other field it carries (a model, an isolation mode), so
/// nothing is left out.
fn task_lines(input: &Value) -> Vec<String> {
    const FIRST: [&str; 3] = ["description", "subagent_type", "prompt"];
    let first = FIRST.iter().filter_map(|k| input.get(*k).map(|v| (*k, v)));
    let rest = fields(input).filter(|(k, _)| !FIRST.contains(&k.as_str())).map(|(k, v)| (k.as_str(), v));
    let lines: Vec<String> = first.chain(rest).flat_map(|(k, v)| key_value_lines(k, &value_text(v))).collect();
    if lines.is_empty() {
        vec!["(no input)".to_string()]
    } else {
        lines
    }
}

fn bash_review(input: &Value) -> Review {
    let Some(command) = str_field(input, "command") else { return Review { lines: all_fields(input), ..Review::default() } };
    // Options such as `dangerouslyDisableSandbox` change what the command may do.
    let notes = fields(input)
        .filter(|(k, _)| !matches!(k.as_str(), "command" | "description" | "timeout"))
        .map(|(k, v)| visible(&format!("{k}={}", value_text(v))))
        .collect();
    Review { notes, lines: text_lines(command) }
}

fn write_lines(input: &Value, project: Option<&Path>) -> Vec<String> {
    let path = visible(&shorten_path(str_field(input, "file_path").unwrap_or("?"), project));
    let content = str_field(input, "content").unwrap_or("");
    let mut lines = vec![format!("{path} ({} bytes)", content.len())];
    if !content.is_empty() {
        lines.extend(text_lines(content).into_iter().map(|l| format!("  {l}")));
    }
    lines
}

/// The `- old` / `+ new` lines of one edit.
fn change_lines(input: &Value) -> Vec<String> {
    let side = |key: &str, mark: &str| -> Vec<String> {
        str_field(input, key).map_or_else(Vec::new, |t| text_lines(t).into_iter().map(|l| format!("{mark} {l}")).collect())
    };
    let mut lines = side("old_string", "-");
    lines.extend(side("new_string", "+"));
    lines
}

fn edit_lines(input: &Value, project: Option<&Path>) -> Vec<String> {
    let all = if input.get("replace_all").and_then(Value::as_bool) == Some(true) { " (every occurrence)" } else { "" };
    let mut lines = vec![format!("{}{all}", visible(&shorten_path(str_field(input, "file_path").unwrap_or("?"), project)))];
    lines.extend(change_lines(input));
    lines
}

fn multi_edit_lines(input: &Value, project: Option<&Path>) -> Vec<String> {
    let edits = input.get("edits").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let mut lines = vec![format!("{} ({} edits)", visible(&shorten_path(str_field(input, "file_path").unwrap_or("?"), project)), edits.len())];
    for (i, edit) in edits.iter().enumerate() {
        lines.push(format!("edit {}:", i + 1));
        lines.extend(change_lines(edit));
    }
    lines
}

fn notebook_lines(input: &Value, project: Option<&Path>) -> Vec<String> {
    let mut lines = vec![visible(&shorten_path(str_field(input, "notebook_path").unwrap_or("?"), project))];
    let details: Vec<String> = ["cell_id", "cell_number", "cell_type", "edit_mode"]
        .iter()
        .filter_map(|k| input.get(*k).map(|v| visible(&format!("{k}={}", value_text(v)))))
        .collect();
    if !details.is_empty() {
        lines.push(details.join(" "));
    }
    if let Some(source) = str_field(input, "new_source") {
        lines.extend(text_lines(source).into_iter().map(|l| format!("+ {l}")));
    }
    lines
}
