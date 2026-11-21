//! Reading and writing the settings files Claude Code reads, and the rules already saved there.
//!
//! Claude Code reads `<project>/.claude/settings.local.json` itself, so "always allow"
//! and `/permissions` only have to keep that file right: merge into what is there (every
//! key this module does not know is kept), never write a file that could not be read
//! back, never duplicate a rule, replace the file atomically and keep it private (0600).

use std::path::{Path, PathBuf};


use crate::approval::domain::always_plan::Always;
use crate::approval::domain::grantable::Places;
use crate::approval::domain::listing::Listing;
use crate::approval::domain::rule_policy::rules_for;
use crate::shared::application::host::Host;
use crate::approval::domain::settings_document::{local_settings_path, user_settings_path, List, SettingsError};
use crate::approval::infrastructure::settings_json::Settings;

/// Whether the project's local settings must be treated as not the user's own: committed to
/// the repository, or git failed in a way that could be the repository's doing (a folder owned
/// by someone else, "dubious ownership", or a damaged index). Only the failures below, which
/// say nothing about the repository, leave the file as the user's own: not under git at all, a
/// locked index (another git running), a missing folder, or no git program.
pub fn local_settings_tracked(host: &dyn Host, project: &Path) -> bool {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(host.no_repo_programs(project))
        .args(["ls-files", "--error-unmatch", "--", crate::approval::domain::settings_document::LOCAL_SETTINGS])
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::null())
        .output();
    match output {
        Ok(out) => crate::approval::domain::rule_policy::tracked_from_git(out.status.code(), &String::from_utf8_lossy(&out.stderr)),
        Err(_) => false,
    }
}

/// The rules that already allow this call, when the person saved every one of them before
/// (project or user settings): the call is then allowed without asking again. Claude Code
/// matches rules itself too, but not always the way jev-code derives them (a `timeout 900`
/// in front, a chain), so a call it still asks about is checked here. A call asked about
/// because of a path outside the allowed folders is never answered from rules alone.
pub fn remembered(host: &dyn Host, tool: &str, input: &crate::approval::domain::tool_input::ToolInput, blocked_path: Option<&str>, places: &Places) -> Option<Vec<String>> {
    remembered_with_local_trust(tool, input, blocked_path, places, !local_settings_tracked(host, &places.project))
}

fn remembered_with_local_trust(tool: &str, input: &crate::approval::domain::tool_input::ToolInput, blocked_path: Option<&str>, places: &Places, local_trusted: bool) -> Option<Vec<String>> {
    if blocked_path.is_some() {
        return None;
    }
    let rules = rules_for(tool, input, None, places).ok()?;
    let mut allowed = if local_trusted {
        load(&local_settings_path(&places.project)).map(|s| s.list(List::Allow)).unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Some(home) = &places.home {
        allowed.extend(load(&user_settings_path(home)).map(|s| s.list(List::Allow)).unwrap_or_default());
    }
    (!rules.is_empty() && rules.iter().all(|r| allowed.contains(r))).then_some(rules)
}

/// Save what `always` says into the project's local settings: the rule and the folder in ONE
/// write, so either both are there afterwards or neither is (a rule without its folder would
/// be asked about again; a folder without its rule is more than was approved). False when
/// everything was already there.
pub fn save_always(project: &Path, always: &Always) -> Result<bool, SettingsError> {
    update(&local_settings_path(project), |settings| {
        // Every rule is tried (no short-circuit), so all of them end up in the file.
        #[allow(clippy::unnecessary_fold)] // `any` would stop at the first rule already there.
        let rule_added = always.rules.iter().fold(false, |added, rule| settings.add(List::Allow, rule) || added);
        let folder_added = always.folder.as_deref().is_some_and(|dir| settings.add(List::Directories, &dir.to_string_lossy()));
        rule_added || folder_added
    })
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

/// The settings files Claude Code reads, as the approval ports see them.
pub struct ClaudeSettingsFiles {
    /// Git's safety options for the "is the local file committed?" check.
    pub host: std::sync::Arc<dyn Host>,
}

impl crate::approval::application::ports::RememberedRules for ClaudeSettingsFiles {
    fn remembered(&self, tool: &str, input: &crate::approval::domain::tool_input::ToolInput, blocked_path: Option<&str>, places: &Places) -> Option<Vec<String>> {
        remembered(&*self.host, tool, input, blocked_path, places)
    }

    fn save_always(&self, project: &Path, always: &Always) -> Result<bool, SettingsError> {
        save_always(project, always)
    }
}

impl crate::approval::application::ports::PermissionsEditor for ClaudeSettingsFiles {
    fn add_allow_rule(&self, project: &Path, rule: &str) -> Result<bool, SettingsError> {
        add_allow_rule(project, rule)
    }

    fn remove_allow_rule(&self, project: &Path, rule: &str) -> Result<bool, SettingsError> {
        remove_allow_rule(project, rule)
    }

    fn add_directory(&self, project: &Path, dir: &Path) -> Result<bool, SettingsError> {
        add_directory(project, dir)
    }

    fn listing(&self, project: &Path, home: Option<&Path>) -> Listing {
        listing(project, home)
    }
}

#[cfg(test)]
mod tests;
