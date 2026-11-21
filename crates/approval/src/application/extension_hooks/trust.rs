//! Trust in a project's hooks and Claude settings: the `trusted-projects` file in the user directory.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{parse_defs, HookDef};
use jev_shared::domain::sha256;

pub(super) fn trusted_file(user: &Path) -> PathBuf {
    user.join("trusted-projects")
}

pub fn is_trusted(user: &Path, project: &Path) -> bool {
    std::fs::read_to_string(trusted_file(user)).is_ok_and(|t| t.lines().any(|line| trusted_path(line) == project))
}

pub(super) fn trusted_path(line: &str) -> &Path {
    Path::new(line.split_once('\t').map_or_else(|| line.trim(), |(path, _)| path))
}

/// A trust line is `path`, `hooks hash` and `settings hash`, tab separated. Older lines have
/// fewer columns: the path alone (the first versions) or the path and hooks hash.
pub(super) fn columns(line: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut parts = line.splitn(3, '\t');
    (parts.next().unwrap_or(""), parts.next(), parts.next())
}

pub(super) fn has_hash(entries: &str, project: &Path, hash: &str) -> bool {
    entries.lines().any(|line| {
        let (path, saved, _) = columns(line);
        Path::new(path) == project && saved == Some(hash)
    })
}

/// Hash of the project's `.claude/settings.json` (of nothing when there is none, which no
/// file added later can match). That file can define hooks, `env` and helper commands that
/// Claude Code runs, so it is trusted only as it was when `/hooks trust` was run.
pub(super) fn settings_hash(project: &Path) -> String {
    sha256::hex(&std::fs::read(project.join(".claude/settings.json")).unwrap_or_default())
}

/// Everything in `.claude/settings.json`, as `(what, value)` lines for the person to review
/// before trusting it: each hook command on its own line, `env` one variable per line, and
/// every other key with its value (permissions, helper commands, MCP servers, plugins ...).
/// Empty for an empty file.
pub(super) fn settings_preview(bytes: &[u8]) -> Vec<(String, String)> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_string);
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) else {
        return vec![("settings.json".into(), "is not a JSON object; Claude Code may still read it".into())];
    };
    let mut out = Vec::new();
    for (key, value) in &map {
        match (key.as_str(), value) {
            ("hooks", Value::Object(events)) => {
                for (event, matchers) in events {
                    let label = format!("hook {event}");
                    // Anything that is not the usual `[{matcher, hooks: [{command}]}]` is shown whole.
                    let Some(list) = matchers.as_array() else {
                        out.push((label, matchers.to_string()));
                        continue;
                    };
                    for matcher in list {
                        // Which tools it fires for belongs to the review.
                        // A well-formed entry without a matcher fires for every tool; anything else
                        // is not claimed to.
                        let label = match (matcher.get("matcher"), matcher.is_object()) {
                            (Some(m), _) => format!("{label}[{}]", m.as_str().map_or_else(|| m.to_string(), str::to_string)),
                            (None, true) => format!("{label}[*]"),
                            (None, false) => label.clone(),
                        };
                        match matcher.get("hooks").and_then(Value::as_array) {
                            Some(hooks) => out.extend(hooks.iter().map(|hook| (label.clone(), hook.get("command").map_or_else(|| hook.to_string(), text)))),
                            None => out.push((label.clone(), matcher.to_string())),
                        }
                    }
                }
            }
            ("env", Value::Object(env)) => out.extend(env.iter().map(|(k, v)| ("env".to_string(), format!("{k}={}", text(v))))),
            ("permissions", Value::Object(rules)) => out.extend(rules.iter().map(|(k, v)| (format!("permissions.{k}"), v.to_string()))),
            _ => out.push((key.clone(), text(value))),
        }
    }
    out
}

/// What the project's `.claude/settings.json` holds now (see [`settings_preview`]).
pub fn claude_settings_preview(project: &Path) -> Vec<(String, String)> {
    settings_preview(&std::fs::read(project.join(".claude/settings.json")).unwrap_or_default())
}

/// Whether the project's `.claude/settings.json` is the one that was trusted.
pub fn claude_settings_trusted(user: &Path, project: &Path) -> bool {
    let current = settings_hash(project);
    std::fs::read_to_string(trusted_file(user)).is_ok_and(|entries| {
        entries.lines().any(|line| {
            let (path, _, saved) = columns(line);
            Path::new(path) == project && saved == Some(current.as_str())
        })
    })
}

/// [`claude_settings_trusted`] for the user directory this process runs under.
pub fn project_settings_trusted(project: &Path) -> bool {
    crate::application::extensions::Places::from_env(project).user.as_deref().is_some_and(|user| claude_settings_trusted(user, project))
}

/// Remember the current project's hooks. `Ok(false)`: they were already trusted.
pub fn trust_project(user: &Path, project: &Path) -> std::io::Result<bool> {
    trust_project_with_preview(user, project).map(|(newly, _)| newly)
}

/// Remember the current hooks and return the commands that were approved.
pub fn trust_project_with_preview(user: &Path, project: &Path) -> std::io::Result<(bool, Vec<HookDef>)> {
    trust_project_full(user, project).map(|(newly, defs, _)| (newly, defs))
}

/// The outcome of [`trust_project_full`]: newly trusted, the hooks, and the settings lines approved.
pub type TrustedWithSettings = (bool, Vec<HookDef>, Vec<(String, String)>);

/// As [`trust_project_with_preview`], with the lines of `.claude/settings.json` that were
/// approved, read from the same bytes that were hashed.
pub fn trust_project_full(user: &Path, project: &Path) -> std::io::Result<TrustedWithSettings> {
    let path = project.to_str().filter(|path| !path.contains(['\t', '\n', '\r']))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "project path cannot be stored"))?;
    let hooks_path = project.join(".jev/hooks.json");
    // No hooks file is fine: the project can still be trusted (for its Claude settings).
    // The hash of nothing then matches no file a later change could add.
    let bytes = match std::fs::read(&hooks_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let hash = sha256::hex(&bytes);
    let settings_bytes = std::fs::read(project.join(".claude/settings.json")).unwrap_or_default();
    let settings = sha256::hex(&settings_bytes);
    let preview = settings_preview(&settings_bytes);
    let defs = parse_defs(&hooks_path, &bytes, &mut Vec::new());
    let entries = match std::fs::read_to_string(trusted_file(user)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let matching: Vec<&str> = entries.lines().filter(|line| trusted_path(line) == project).collect();
    if matching.len() == 1 && has_hash(matching[0], project, &hash) && columns(matching[0]).2 == Some(settings.as_str()) {
        return Ok((false, defs, preview));
    }
    std::fs::create_dir_all(user)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(trusted_file(user))?;
    for line in entries.lines().filter(|line| trusted_path(line) != project) {
        writeln!(file, "{line}")?;
    }
    writeln!(file, "{path}\t{hash}\t{settings}")?;
    Ok((true, defs, preview))
}
