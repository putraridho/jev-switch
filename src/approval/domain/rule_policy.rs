//! The allow rule that would stop Claude Code asking again, for each kind of tool call.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use crate::approval::domain::bash_rule::{bash_rule, bash_rules};
use crate::approval::domain::grantable::{grantable_file, grantable_folder, Places};
use crate::approval::domain::refusal::Refusal;
use crate::approval::domain::tool_input::str_field;
use crate::approval::domain::visible_text::is_invisible;


/// What git says when it fails for a reason that is not the repository's doing.
const HARMLESS_GIT_FAILURES: [&str; 3] = ["not a git repository", "index.lock", "cannot change to"];

/// The decision of `claude_settings_file::local_settings_tracked` from git's exit code and what it said.
pub fn tracked_from_git(code: Option<i32>, stderr: &str) -> bool {
    match code {
        Some(0) => true,
        Some(1) => false,
        _ => !HARMLESS_GIT_FAILURES.iter().any(|text| stderr.contains(text)),
    }
}

/// Claude Code's spelling of an absolute path in a rule: `//` and the path.
pub(super) fn abs_path_rule(path: &Path) -> String {
    format!("/{}", path.display())
}

/// `path` as long as it names one place: no wildcards, `..`, parentheses or control characters.
pub(super) fn literal_path(path: &str) -> Result<&Path, Refusal> {
    let odd = |c: char| c.is_control() || is_invisible(c) || matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '(' | ')');
    let p = Path::new(path);
    if path.is_empty() || path.chars().any(odd) || p.components().any(|c| matches!(c, Component::ParentDir)) {
        Err(Refusal::UnsafePath)
    } else {
        Ok(p)
    }
}

/// The host of a URL when it is a plain host name.
pub(super) fn plain_host(url: &str) -> Result<&str, Refusal> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    let plain = !host.is_empty()
        && !host.starts_with(['.', '-'])
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if plain {
        Ok(host)
    } else {
        Err(Refusal::BadHost)
    }
}

/// The allow rule that would let Claude Code run this call without asking again, in
/// Claude Code's rule syntax, or why there is none narrower than "everything".
pub fn rule_for(tool: &str, input: &Value, blocked_path: Option<&str>, places: &Places) -> Result<String, Refusal> {
    match tool {
        "Bash" => bash_rule(str_field(input, "command").ok_or(Refusal::NoRule)?),
        // Claude Code checks every file-editing tool (Write, Edit, MultiEdit, NotebookEdit)
        // against `Edit(path)` rules only; it accepts a `Write(path)` rule but never matches a
        // write against it. Path anchors: `//path` is absolute, `~/path` the home folder,
        // `/path` relative to the settings source (the project), `path` the current folder.
        // https://code.claude.com/docs/en/permissions ("Read and Edit", and "`Write(path)`,
        // `Glob(path)`, and `Grep(path)` rules don't match"). The rule is always `//absolute`,
        // with a relative `file_path` resolved against the project the tool runs in.
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let key = if tool == "NotebookEdit" { "notebook_path" } else { "file_path" };
            let path = literal_path(str_field(input, key).or(blocked_path).ok_or(Refusal::NoRule)?)?;
            // Collecting the components drops a `./` the path started with.
            let path: PathBuf = places.project.join(path).components().collect();
            grantable_file(&path, places)?;
            Ok(format!("Edit({})", abs_path_rule(&path)))
        }
        "Read" | "Grep" | "Glob" | "LS" => {
            let named = blocked_path.or_else(|| str_field(input, "file_path")).or_else(|| str_field(input, "path"));
            let target = literal_path(named.ok_or(Refusal::NoRule)?)?;
            let target = if target.is_absolute() { target.to_path_buf() } else { places.project.join(target) };
            if target.starts_with(&places.project) {
                return Ok(format!("Read({})", abs_path_rule(&target)));
            }
            // Outside the project the whole folder is what the user is vouching for; a file
            // read has its folder, a search or listing already names one. The folder is
            // judged like any other permanent grant: reading `~/.bashrc` must not become
            // "read everything in the home folder".
            let dir = if tool == "Read" { target.parent().ok_or(Refusal::FolderTooBroad)?.to_path_buf() } else { target };
            grantable_folder(&dir, places)?;
            Ok(format!("Read({}/**)", abs_path_rule(&dir)))
        }
        "WebFetch" => Ok(format!("WebFetch(domain:{})", plain_host(str_field(input, "url").ok_or(Refusal::NoRule)?)?)),
        "WebSearch" => Ok("WebSearch".into()),
        t if t.starts_with("mcp__") && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) => Ok(t.to_string()),
        _ => Err(Refusal::NoRule),
    }
}

/// [`rule_for`] without the reason.
pub fn allow_rule(tool: &str, input: &Value, blocked_path: Option<&str>, places: &Places) -> Option<String> {
    rule_for(tool, input, blocked_path, places).ok()
}

/// [`rule_for`], except that a Bash chain (`a && b | c`, `2>&1`, `>/dev/null`) gets one rule
/// per simple command when every one of them has a safe rule (see [`bash_rules`]).
pub fn rules_for(tool: &str, input: &Value, blocked_path: Option<&str>, places: &Places) -> Result<Vec<String>, Refusal> {
    match tool {
        "Bash" => bash_rules(str_field(input, "command").ok_or(Refusal::NoRule)?),
        _ => rule_for(tool, input, blocked_path, places).map(|rule| vec![rule]),
    }
}
