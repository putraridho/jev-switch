//! Claude Code's allow-rule syntax: checking a rule and matching a call against rules.

use serde_json::Value;
use crate::approval::domain::shell_words::split_chain;
use crate::approval::domain::tool_input::str_field;

// ------------------------------------------------------------------ the rule syntax

/// A rule as Claude Code reads one: a tool name, or `Tool(content)`. Rejects what would
/// be written to the file and then never match (or break the parser).
pub fn validate_rule(rule: &str) -> Result<String, String> {
    let rule = rule.trim();
    let bad = |why: &str| Err(format!("not a permission rule ({why}); examples: Bash(cargo test:*), Read(//tmp/**), WebFetch(domain:docs.rs)"));
    let (tool, content) = match rule.split_once('(') {
        Some((tool, rest)) => match rest.strip_suffix(')') {
            Some(content) => (tool, Some(content)),
            None => return bad("missing the closing parenthesis"),
        },
        None => (rule, None),
    };
    // Claude Code skips a rule whose tool name starts in lower case (`all`), except MCP tools.
    if matches!(rule.to_ascii_lowercase().as_str(), "all" | "any" | "everything" | "*" | "yes") {
        return Err("not a permission rule: no rule allows everything. To be asked less, press Shift-Tab or use /mode auto; to allow one tool or command, name it, e.g. Bash(cargo test:*), Edit or WebFetch(domain:docs.rs)".to_string());
    }
    let first_ok = tool.starts_with("mcp__") || tool.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let name_ok = first_ok && tool.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '-' | '.'));
    if !name_ok {
        return bad("it should start with a tool name such as Bash, Read, Write or WebFetch");
    }
    if content.is_some_and(|c| c.trim().is_empty() || c.contains('\n')) {
        return bad("the part in parentheses is empty or has a line break");
    }
    Ok(rule.to_string())
}

// ------------------------------------------------------- rules given on the command line

/// The rules of a `CODE_ALLOWED_TOOLS` list, as Claude Code's `--allowedTools` reads it:
/// separated by spaces or commas, except inside parentheses (`Bash(cargo test) Edit`).
pub fn split_rules(list: &str) -> Vec<String> {
    let (mut rules, mut rule, mut depth) = (Vec::new(), String::new(), 0usize);
    for c in list.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && (c == ',' || c.is_whitespace()) {
            if !rule.is_empty() {
                rules.push(std::mem::take(&mut rule));
            }
        } else {
            rule.push(c);
        }
    }
    if !rule.is_empty() {
        rules.push(rule);
    }
    rules
}

/// Whether `rules` (Claude Code's allow-rule syntax, e.g. from `CODE_ALLOWED_TOOLS`) allow this
/// call without asking. Claude Code applies such rules itself; a CLI that asks jev-code about
/// every call instead (Antigravity) gets the same answer here. Only forms whose meaning is
/// certain match: a bare tool name (`Edit`), and for Bash an exact command (`Bash(cargo test)`),
/// a prefix (`Bash(git status:*)`) or a `*` wildcard (`Bash(python3 *)`). Every simple command
/// of a chain must match, and a command [`split_chain`] refuses (`$`, backticks, subshells, a
/// redirection to a file) never does. Anything else (`Edit(src/**)`) is left to asking.
pub fn allowed_by(rules: &[String], tool: &str, input: &Value) -> bool {
    if rules.iter().any(|r| r == tool) {
        return true;
    }
    if tool != "Bash" {
        return false;
    }
    let specs: Vec<&str> = rules.iter().filter_map(|r| r.strip_prefix("Bash(")?.strip_suffix(')')).collect();
    let Some(command) = str_field(input, "command") else { return false };
    let Ok(segments) = split_chain(command) else { return false };
    !specs.is_empty()
        && segments.iter().all(|segment| {
            let words: Vec<&str> = segment.iter().map(|w| w.text.as_str()).collect();
            specs.iter().any(|spec| bash_spec_matches(spec, &words))
        })
}

/// One simple command's words against the part of a `Bash(...)` rule in parentheses.
fn bash_spec_matches(spec: &str, words: &[&str]) -> bool {
    if let Some(prefix) = spec.strip_suffix(":*") {
        let want: Vec<&str> = prefix.split_whitespace().collect();
        return !want.is_empty() && words.len() >= want.len() && words[..want.len()] == want[..];
    }
    if spec.contains('*') {
        return wildcard_matches(spec, &words.join(" "));
    }
    spec.split_whitespace().eq(words.iter().copied())
}

/// `*` matches any run of characters (spaces too); everything else matches itself.
fn wildcard_matches(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}
