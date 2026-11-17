//! Tool permissions for the Claude Code backends: what a request is called on screen, the
//! allow rule that would stop Claude Code asking again, and the settings file that holds
//! the rules.
//!
//! Claude Code reads `<project>/.claude/settings.local.json` itself, so "always allow"
//! and `/permissions` only have to keep that file right: merge into what is there (every
//! key this module does not know is kept), never write a file that could not be read
//! back, never duplicate a rule, replace the file atomically and keep it private (0600).
//! Everything here is pure or touches only the one settings file, so it is unit tested
//! without Claude Code.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::backend::claude::telling_field;

/// Where the project's own rules live, relative to the project folder.
pub const LOCAL_SETTINGS: &str = ".claude/settings.local.json";

/// Longest single line of a request summary kept; the console cuts it to the width.
const SUMMARY_MAX_CHARS: usize = 400;

// ------------------------------------------------------------------- the request

/// What the user is shown for one tool call: the command, or the path (and size for a
/// whole-file write; the contents themselves are never printed). One line.
pub fn describe(tool: &str, input: &Value) -> String {
    let text = match tool {
        "Write" => {
            let path = str_field(input, "file_path").unwrap_or("?");
            let bytes = str_field(input, "content").map_or(0, str::len);
            format!("{path} ({bytes} bytes)")
        }
        "Bash" => {
            let command = str_field(input, "command").unwrap_or("").trim();
            let mut lines = command.lines();
            let first = lines.next().unwrap_or("");
            match lines.count() {
                0 => first.to_string(),
                more => format!("{first} \u{2026} (+{more} lines)"),
            }
        }
        _ => telling_field(input).unwrap_or("").to_string(),
    };
    text.chars().take(SUMMARY_MAX_CHARS).collect()
}

fn str_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

// ------------------------------------------------------------------ rule derivation

/// Commands that run other commands: allowing `bash -c:*` would allow everything.
const WRAPPERS: [&str; 19] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "env", "sudo", "doas", "su", "eval", "exec", "xargs", "nohup", "time",
    "watch", "ssh", "command", "builtin",
];

fn is_plain_program(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '+' | '-'))
}

fn is_subcommand(word: &str) -> bool {
    word.len() <= 24
        && word.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// `Bash(cargo test:*)` for `cargo test --quiet`: the program and, when the next word
/// looks like a subcommand (not a flag or a path), that too. `None` for shell wrappers
/// and anything that does not start with a plain program name.
fn bash_rule(command: &str) -> Option<String> {
    let mut words = command.split_whitespace();
    let program = words.next().filter(|w| is_plain_program(w) && !WRAPPERS.contains(w))?;
    Some(match words.next().filter(|w| is_subcommand(w)) {
        Some(sub) => format!("Bash({program} {sub}:*)"),
        None => format!("Bash({program}:*)"),
    })
}

/// Claude Code's spelling of an absolute path in a rule: `//` and the path.
fn abs_path_rule(path: &Path) -> String {
    format!("/{}", path.display())
}

/// The allow rule that would let Claude Code run this call without asking again, in
/// Claude Code's rule syntax. `None` when no rule narrower than "everything" exists
/// (shell wrappers, unknown tools), so the user is not offered "always" for it.
pub fn allow_rule(tool: &str, input: &Value, blocked_path: Option<&str>, project: &Path) -> Option<String> {
    match tool {
        "Bash" => bash_rule(str_field(input, "command")?),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let path = str_field(input, "file_path").or(blocked_path)?;
            Some(format!("{tool}({})", abs_path_rule(Path::new(path))))
        }
        "Read" | "Grep" | "Glob" | "LS" => {
            let named = blocked_path.or_else(|| str_field(input, "file_path")).or_else(|| str_field(input, "path"));
            let target = Path::new(named?);
            let target = if target.is_absolute() { target.to_path_buf() } else { project.join(target) };
            if target.starts_with(project) {
                return Some(format!("Read({})", abs_path_rule(&target)));
            }
            // Outside the project the whole folder is what the user is vouching for; a file
            // read has its folder, a search or listing already names one.
            let dir = if tool == "Read" { target.parent()?.to_path_buf() } else { target };
            Some(format!("Read({}/**)", abs_path_rule(&dir)))
        }
        "WebFetch" => {
            let url = str_field(input, "url")?;
            let host = url.split_once("://").map_or(url, |(_, rest)| rest).split(['/', '?', '#']).next()?;
            let host = host.rsplit('@').next()?.split(':').next()?;
            (!host.is_empty()).then(|| format!("WebFetch(domain:{host})"))
        }
        "WebSearch" => Some("WebSearch".into()),
        t if t.starts_with("mcp__") => Some(t.to_string()),
        _ => None,
    }
}

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
    let name_ok = tool.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && tool.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '*' | '-' | '.'));
    if !name_ok {
        return bad("it should start with a tool name such as Bash, Read, Write or WebFetch");
    }
    if content.is_some_and(|c| c.trim().is_empty() || c.contains('\n')) {
        return bad("the part in parentheses is empty or has a line break");
    }
    Ok(rule.to_string())
}

// ----------------------------------------------------------------- the settings file

/// Why a settings file could not be read or written. The text names the file, so it can
/// be shown as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError(pub String);

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SettingsError {}

pub fn local_settings_path(project: &Path) -> PathBuf {
    project.join(LOCAL_SETTINGS)
}

/// The user-level settings Claude Code also reads (`~/.claude/settings.json`).
pub fn user_settings_path(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

/// The two lists of the `permissions` object this module edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Allow,
    Directories,
}

impl List {
    fn key(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Directories => "additionalDirectories",
        }
    }
}

/// A settings document: any JSON object, with the shape of the part we edit checked.
/// Everything else in it is carried along untouched.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    doc: Value,
}

impl Settings {
    /// An empty document, for a file that does not exist yet.
    pub fn empty() -> Self {
        Self { doc: Value::Object(Map::new()) }
    }

    /// Parse `text`. Refuses anything that would have to be overwritten to be edited:
    /// invalid JSON, a root that is not an object, `permissions` that is not an object,
    /// or one of our two lists that is not a list of strings.
    pub fn parse(text: &str, file: &Path) -> Result<Self, SettingsError> {
        let err = |why: String| SettingsError(format!("{}: {why}; left as it is", file.display()));
        let doc: Value = serde_json::from_str(text).map_err(|e| err(format!("not valid JSON ({e})")))?;
        let Some(root) = doc.as_object() else { return Err(err("the top level is not a JSON object".into())) };
        if let Some(permissions) = root.get("permissions") {
            let Some(permissions) = permissions.as_object() else { return Err(err("\"permissions\" is not an object".into())) };
            for list in [List::Allow, List::Directories] {
                let ok = permissions
                    .get(list.key())
                    .is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string)));
                if !ok {
                    return Err(err(format!("\"permissions.{}\" is not a list of strings", list.key())));
                }
            }
        }
        Ok(Self { doc })
    }

    /// The entries of one list, in file order.
    pub fn list(&self, list: List) -> Vec<String> {
        self.doc["permissions"][list.key()]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    fn entries_mut(&mut self, list: List) -> &mut Vec<Value> {
        let root = self.doc.as_object_mut().expect("checked in parse");
        let permissions = root.entry("permissions").or_insert_with(|| json!({}));
        let permissions = permissions.as_object_mut().expect("checked in parse");
        permissions.entry(list.key()).or_insert_with(|| json!([])).as_array_mut().expect("checked in parse")
    }

    /// Add `entry` unless it is there. True when the document changed.
    pub fn add(&mut self, list: List, entry: &str) -> bool {
        if self.list(list).iter().any(|e| e == entry) {
            return false;
        }
        self.entries_mut(list).push(Value::String(entry.to_string()));
        true
    }

    /// Remove every copy of `entry`. True when the document changed. An emptied list is
    /// left as an empty list, never deleted: the user may have put it there.
    pub fn remove(&mut self, list: List, entry: &str) -> bool {
        if !self.list(list).iter().any(|e| e == entry) {
            return false;
        }
        self.entries_mut(list).retain(|v| v.as_str() != Some(entry));
        true
    }

    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.doc).unwrap_or_else(|_| "{}".into());
        text.push('\n');
        text
    }
}

/// Read a settings file: a missing file is an empty document, a file that cannot be read
/// or parsed is an error (and is never replaced).
pub fn load(path: &Path) -> Result<Settings, SettingsError> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Settings::empty()),
        Ok(text) => Settings::parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::empty()),
        Err(e) => Err(SettingsError(format!("{}: cannot read it ({e})", path.display()))),
    }
}

/// Write `text` to `path` through a private temporary file in the same folder and a
/// rename, so a reader (Claude Code) sees the old file or the new one, never half of it.
fn write_private_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map_or_else(|| "settings".into(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Load `path`, apply `edit`, and write the file back only when `edit` changed something.
/// Returns whether it did.
fn update(path: &Path, edit: impl FnOnce(&mut Settings) -> bool) -> Result<bool, SettingsError> {
    let mut settings = load(path)?;
    if !edit(&mut settings) {
        return Ok(false);
    }
    write_private_atomic(path, &settings.to_text())
        .map_err(|e| SettingsError(format!("{}: cannot write it ({e})", path.display())))?;
    Ok(true)
}

/// Add an allow rule to the project's local settings. False when it was already there.
pub fn add_allow_rule(project: &Path, rule: &str) -> Result<bool, SettingsError> {
    update(&local_settings_path(project), |s| s.add(List::Allow, rule))
}

/// Remove an allow rule from the project's local settings. False when it was not there.
pub fn remove_allow_rule(project: &Path, rule: &str) -> Result<bool, SettingsError> {
    update(&local_settings_path(project), |s| s.remove(List::Allow, rule))
}

/// Add an additional directory (Claude Code may then read and edit inside it). False when
/// it was already there.
pub fn add_directory(project: &Path, dir: &Path) -> Result<bool, SettingsError> {
    update(&local_settings_path(project), |s| s.add(List::Directories, &dir.to_string_lossy()))
}

/// The folder a `/permissions dir` argument names: `~` and a relative path resolved, and it
/// has to exist as a folder, so a typo is not saved as a rule that never matches.
pub fn resolve_dir(arg: &str, project: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let arg = arg.trim();
    let path = match (arg.strip_prefix("~/"), arg, home) {
        (Some(rest), _, Some(home)) => home.join(rest),
        (None, "~", Some(home)) => home.to_path_buf(),
        _ => project.join(arg),
    };
    let dir = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!("{} is not a folder", dir.display()))
    }
}

/// What `/permissions` shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub local_path: PathBuf,
    pub allow: Vec<String>,
    pub directories: Vec<String>,
    pub user_path: Option<PathBuf>,
    pub user_allow: Vec<String>,
    /// Files that could not be read, in words.
    pub problems: Vec<String>,
}

/// The project's rules and folders and, read-only, the user-level allow list.
pub fn listing(project: &Path, home: Option<&Path>) -> Listing {
    let local_path = local_settings_path(project);
    let mut problems = Vec::new();
    let mut read = |path: &Path| match load(path) {
        Ok(s) => s,
        Err(e) => {
            problems.push(e.0);
            Settings::empty()
        }
    };
    let local = read(&local_path);
    let user_path = home.map(user_settings_path);
    let user = user_path.as_deref().map(&mut read).unwrap_or_else(Settings::empty);
    Listing {
        allow: local.list(List::Allow),
        directories: local.list(List::Directories),
        user_allow: user.list(List::Allow),
        local_path,
        user_path,
        problems,
    }
}

// ------------------------------------------------------------------- the command

/// What `/permissions` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionsCmd {
    List,
    Allow(String),
    Remove(String),
    Dir(String),
}

pub const USAGE: &str = "usage: /permissions [allow <rule> | remove <rule> | dir <path>]";

/// The text after `/permissions`. A rule has spaces in it (`Bash(cargo test:*)`), so the
/// rest of the line is the rule.
pub fn parse_args(rest: &str) -> Result<PermissionsCmd, String> {
    let rest = rest.trim();
    let (head, arg) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(h, a)| (h, a.trim()));
    match (head.to_ascii_lowercase().as_str(), arg) {
        ("", _) | ("list", "") => Ok(PermissionsCmd::List),
        ("allow" | "add", rule) if !rule.is_empty() => validate_rule(rule).map(PermissionsCmd::Allow),
        ("remove" | "rm" | "delete", rule) if !rule.is_empty() => Ok(PermissionsCmd::Remove(rule.to_string())),
        ("dir", path) if !path.is_empty() => Ok(PermissionsCmd::Dir(path.to_string())),
        _ => Err(USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> &'static Path {
        Path::new("/work/app")
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-perm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn requests_are_described_in_one_line_without_file_contents() {
        assert_eq!(describe("Bash", &json!({"command": "cargo test --quiet"})), "cargo test --quiet");
        assert_eq!(describe("Bash", &json!({"command": "echo a\necho b\necho c"})), "echo a \u{2026} (+2 lines)");
        assert_eq!(
            describe("Write", &json!({"file_path": "/work/app/a.txt", "content": "secret text"})),
            "/work/app/a.txt (11 bytes)"
        );
        assert_eq!(describe("Read", &json!({"file_path": "/etc/hosts"})), "/etc/hosts");
        assert_eq!(describe("Task", &json!({})), "");
        assert_eq!(describe("Bash", &json!({"command": "x".repeat(1000)})).chars().count(), SUMMARY_MAX_CHARS);
    }

    #[test]
    fn bash_rules_use_the_program_and_a_subcommand() {
        let rule = |c: &str| allow_rule("Bash", &json!({"command": c}), None, project());
        assert_eq!(rule("cargo test --quiet").as_deref(), Some("Bash(cargo test:*)"));
        assert_eq!(rule("git commit -m \"x\"").as_deref(), Some("Bash(git commit:*)"));
        assert_eq!(rule("grep -rn foo src").as_deref(), Some("Bash(grep:*)"));
        assert_eq!(rule("ls").as_deref(), Some("Bash(ls:*)"));
        assert_eq!(rule("./run.sh ../x").as_deref(), Some("Bash(./run.sh:*)"));
        assert_eq!(rule("cargo   build").as_deref(), Some("Bash(cargo build:*)"));
    }

    #[test]
    fn no_rule_is_offered_where_it_would_allow_everything() {
        let rule = |c: &str| allow_rule("Bash", &json!({"command": c}), None, project());
        for command in ["bash -c 'rm -rf x'", "sudo ls", "env FOO=1 make", "FOO=1 make", "xargs rm", "", "   ", "$(evil) x", "\"quoted\" x"] {
            assert_eq!(rule(command), None, "{command:?}");
        }
        assert_eq!(allow_rule("Task", &json!({}), None, project()), None);
        assert_eq!(allow_rule("Bash", &json!({}), None, project()), None);
    }

    #[test]
    fn file_rules_use_the_absolute_path_spelling() {
        assert_eq!(
            allow_rule("Write", &json!({"file_path": "/work/app/.claude/settings.local.json"}), None, project()).as_deref(),
            Some("Write(//work/app/.claude/settings.local.json)")
        );
        assert_eq!(allow_rule("Edit", &json!({"file_path": "/work/app/a.rs"}), None, project()).as_deref(), Some("Edit(//work/app/a.rs)"));
        // A read outside the project allows its folder; a search names the folder itself.
        assert_eq!(
            allow_rule("Read", &json!({"file_path": "/etc/nginx/nginx.conf"}), None, project()).as_deref(),
            Some("Read(//etc/nginx/**)")
        );
        assert_eq!(allow_rule("Grep", &json!({"pattern": "x", "path": "/var/log"}), None, project()).as_deref(), Some("Read(//var/log/**)"));
        assert_eq!(allow_rule("Read", &json!({"file_path": "/work/app/a.rs"}), None, project()).as_deref(), Some("Read(//work/app/a.rs)"));
        // The blocked path Claude Code reports wins over the input.
        assert_eq!(
            allow_rule("Read", &json!({"file_path": "x"}), Some("/opt/data/f.txt"), project()).as_deref(),
            Some("Read(//opt/data/**)")
        );
    }

    #[test]
    fn web_and_mcp_rules() {
        let web = |u: &str| allow_rule("WebFetch", &json!({"url": u}), None, project());
        assert_eq!(web("https://docs.rs/serde/latest?x=1").as_deref(), Some("WebFetch(domain:docs.rs)"));
        assert_eq!(web("http://user@localhost:8080/a").as_deref(), Some("WebFetch(domain:localhost)"));
        assert_eq!(web("https:///").as_deref(), None);
        assert_eq!(allow_rule("mcp__jev__delegate", &json!({}), None, project()).as_deref(), Some("mcp__jev__delegate"));
    }

    #[test]
    fn rule_syntax_is_checked_before_it_is_saved() {
        for ok in ["Bash(cargo test:*)", "Read(//tmp/**)", "WebSearch", "mcp__jev__delegate", "  Write(//a b)  "] {
            assert_eq!(validate_rule(ok).unwrap(), ok.trim());
        }
        for bad in ["", "   ", "Bash(", "Bash()", "Bash( )", "(x)", "9Bash", "Bash(a\nb)", "Ba sh(x)", "Bash(x) y"] {
            assert!(validate_rule(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn settings_round_trip_keeps_unknown_keys() {
        let dir = tmp("roundtrip");
        let path = local_settings_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"model":"x","hooks":{"a":[1,2]},"permissions":{"allow":["Bash(ls:*)"],"deny":["Read(.env)"],"defaultMode":"acceptEdits"}}"#,
        )
        .unwrap();
        assert!(add_allow_rule(&dir, "Bash(cargo test:*)").unwrap());
        assert!(add_directory(&dir, Path::new("/opt/data")).unwrap());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"], "x");
        assert_eq!(v["hooks"], json!({"a": [1, 2]}));
        assert_eq!(v["permissions"]["deny"], json!(["Read(.env)"]));
        assert_eq!(v["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(v["permissions"]["allow"], json!(["Bash(ls:*)", "Bash(cargo test:*)"]));
        assert_eq!(v["permissions"]["additionalDirectories"], json!(["/opt/data"]));
        assert!(remove_allow_rule(&dir, "Bash(ls:*)").unwrap());
        assert_eq!(load(&path).unwrap().list(List::Allow), ["Bash(cargo test:*)"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rule_is_never_added_twice_or_removed_when_absent() {
        let dir = tmp("dupes");
        assert!(add_allow_rule(&dir, "Bash(ls:*)").unwrap(), "creates the file and the folder");
        assert!(!add_allow_rule(&dir, "Bash(ls:*)").unwrap());
        assert!(!remove_allow_rule(&dir, "Bash(nope:*)").unwrap());
        assert_eq!(load(&local_settings_path(&dir)).unwrap().list(List::Allow), ["Bash(ls:*)"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_read_back_is_refused_and_left_alone() {
        let dir = tmp("refuse");
        let path = local_settings_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bad in [
            "{ not json",
            "[]",
            r#"{"permissions":[]}"#,
            r#"{"permissions":{"allow":"Bash"}}"#,
            r#"{"permissions":{"allow":[1]}}"#,
            r#"{"permissions":{"additionalDirectories":{}}}"#,
        ] {
            std::fs::write(&path, bad).unwrap();
            let e = add_allow_rule(&dir, "Bash(ls:*)").unwrap_err();
            assert!(e.0.contains("settings.local.json") && e.0.contains("left as it is"), "{e}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad, "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_and_missing_files_are_empty_settings() {
        let dir = tmp("empty");
        let path = local_settings_path(&dir);
        assert_eq!(load(&path).unwrap(), Settings::empty());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "  \n").unwrap();
        assert_eq!(load(&path).unwrap(), Settings::empty());
        assert!(add_allow_rule(&dir, "WebSearch").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_even_when_it_was_not_and_no_temp_file_is_left() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("mode");
        let path = local_settings_path(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        add_allow_rule(&dir, "WebSearch").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_arguments_resolve_against_the_project_and_must_exist() {
        let dir = tmp("dirs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file"), "x").unwrap();
        assert_eq!(resolve_dir("sub", &dir, None).unwrap(), dir.join("sub").canonicalize().unwrap());
        assert_eq!(resolve_dir("~/sub", &dir, Some(&dir)).unwrap(), dir.join("sub").canonicalize().unwrap());
        assert_eq!(resolve_dir(dir.join("sub").to_str().unwrap(), Path::new("/nowhere"), None).unwrap(), dir.join("sub").canonicalize().unwrap());
        assert!(resolve_dir("missing", &dir, None).is_err());
        assert!(resolve_dir("file", &dir, None).unwrap_err().contains("not a folder"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_listing_reads_the_project_and_the_user_file_and_reports_a_broken_one() {
        let dir = tmp("listing");
        let home = tmp("listing-home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(user_settings_path(&home), r#"{"permissions":{"allow":["Bash(git status:*)"]}}"#).unwrap();
        add_allow_rule(&dir, "Bash(ls:*)").unwrap();
        add_directory(&dir, Path::new("/opt/x")).unwrap();
        let l = listing(&dir, Some(&home));
        assert_eq!((l.allow, l.directories, l.user_allow), (vec!["Bash(ls:*)".into()], vec!["/opt/x".into()], vec!["Bash(git status:*)".into()]));
        assert!(l.problems.is_empty());
        std::fs::write(user_settings_path(&home), "{").unwrap();
        let l = listing(&dir, Some(&home));
        assert!(l.user_allow.is_empty() && l.problems.len() == 1 && l.problems[0].contains("settings.json"), "{l:?}");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_command_line_is_parsed() {
        assert_eq!(parse_args(""), Ok(PermissionsCmd::List));
        assert_eq!(parse_args(" list "), Ok(PermissionsCmd::List));
        assert_eq!(parse_args("allow Bash(cargo test:*)"), Ok(PermissionsCmd::Allow("Bash(cargo test:*)".into())));
        assert_eq!(parse_args("ALLOW   Read(//tmp/**) "), Ok(PermissionsCmd::Allow("Read(//tmp/**)".into())));
        assert_eq!(parse_args("remove Bash(cargo test:*)"), Ok(PermissionsCmd::Remove("Bash(cargo test:*)".into())));
        assert_eq!(parse_args("dir ../other dir"), Ok(PermissionsCmd::Dir("../other dir".into())));
        for bad in ["allow", "remove", "dir", "nope", "list extra", "allow Bash("] {
            assert!(parse_args(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_args("nope"), Err(USAGE.to_string()));
    }
}
