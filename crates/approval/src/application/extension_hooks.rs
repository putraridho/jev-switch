//! Hooks: shell commands run on events of a turn.
//!
//! ```json
//! { "hooks": [
//!   { "event": "pre_tool",  "tool": "Bash|Write", "run": "./scripts/guard.sh", "timeout_secs": 10 },
//!   { "event": "post_tool", "run": "echo done >> /tmp/tools.log" },
//!   { "event": "turn_end",  "run": "notify-send finished" } ] }
//! ```
//!
//! read from `<project>/.jev/hooks.json` and `<config>/hooks.json`. The event is sent to the
//! command as one JSON object on stdin ([`Payload`]) and in `JEV_HOOK_EVENT` / `JEV_HOOK_TOOL`.
//! A `pre_tool` hook that exits with status 2 refuses the tool call: what it wrote on stderr is
//! what the model is told. Any other failure is a warning and the call goes on.
//!
//! A `pre_tool` hook can only refuse a call the CLI asks the host about (an edit or command
//! that its own settings allow without asking never reaches jev-code). Hooks in a project
//! run code that came with the repository, so they stay off until the person trusts the
//! project's current hooks file (`/hooks trust`). Trust is bound to the file's contents:
//! any change to it turns the project's hooks off again until it is trusted anew. The same
//! trust (by path) also lets Claude Code load the project's own `.claude/settings.json`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::application::extensions::Places;
use jev_shared::domain::sha256;

use runner::{clip, run_command, Ran};
use trust::{claude_settings_preview, claude_settings_trusted, has_hash, is_trusted, trusted_file};

pub mod runner;
pub mod trust;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Before a tool call that needs approval.
    PreTool,
    /// After the model made a tool call.
    PostTool,
    TurnStart,
    TurnEnd,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::PreTool => "pre_tool",
            Event::PostTool => "post_tool",
            Event::TurnStart => "turn_start",
            Event::TurnEnd => "turn_end",
        }
    }
}

const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HookDef {
    pub event: Event,
    /// Tool names this applies to, `|` between alternatives; absent or `*` means every tool.
    #[serde(default)]
    pub tool: Option<String>,
    pub run: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct HooksFile {
    #[serde(default)]
    hooks: Vec<HookDef>,
}

/// What a hook is told.
#[derive(Debug, Serialize)]
pub struct Payload<'a> {
    pub event: Event,
    pub tool: Option<&'a str>,
    /// One line about the call: the command, or the file path.
    pub summary: Option<&'a str>,
    pub project: &'a Path,
}

impl HookDef {
    fn matches(&self, event: Event, tool: Option<&str>) -> bool {
        if self.event != event {
            return false;
        }
        match (self.tool.as_deref(), tool) {
            (None | Some("*"), _) => true,
            (Some(pattern), Some(tool)) => pattern.split('|').any(|p| p.trim() == tool),
            (Some(_), None) => false,
        }
    }
}

/// What running the hooks for one event came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Set by a `pre_tool` hook that exited with status 2.
    pub refused: Option<String>,
    /// Warnings: hooks that failed or timed out.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Hooks {
    defs: Vec<HookDef>,
    dir: PathBuf,
    project_start: usize,
    project_hash: Option<String>,
    untrusted_defs: Vec<HookDef>,
    untrusted_settings: Vec<(String, String)>,
    /// The project was trusted before, but its `.claude/settings.json` is new or changed since
    /// (or was trusted by a version that did not record it), so Claude Code does not load it.
    pub settings_changed: bool,
    /// Hooks in the project's own file exist but the project is not trusted, so they were not loaded.
    pub skipped_untrusted: usize,
    /// Problems reading the files, one line each.
    pub problems: Vec<String>,
}

fn read_defs(path: &Path, problems: &mut Vec<String>) -> Vec<HookDef> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    parse_defs(path, &bytes, problems)
}

fn parse_defs(path: &Path, bytes: &[u8], problems: &mut Vec<String>) -> Vec<HookDef> {
    match serde_json::from_slice::<HooksFile>(bytes) {
        Ok(file) => file.hooks.into_iter().filter(|h| !h.run.trim().is_empty()).collect(),
        Err(e) => {
            problems.push(format!("{}: {e}", path.display()));
            Vec::new()
        }
    }
}

impl Hooks {
    pub fn none(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf(), ..Self::default() }
    }

    pub fn load(places: &Places) -> Self {
        let mut hooks = Self::none(&places.project);
        if let Some(user) = &places.user {
            let defs = read_defs(&user.join("hooks.json"), &mut hooks.problems);
            hooks.defs.extend(defs);
        }
        hooks.project_start = hooks.defs.len();
        let path = places.project.join(".jev/hooks.json");
        if let Ok(bytes) = std::fs::read(&path) {
            let project_defs = parse_defs(&path, &bytes, &mut hooks.problems);
            let hash = sha256::hex(&bytes);
            let trusted = places.user.as_deref().and_then(|user| std::fs::read_to_string(trusted_file(user)).ok())
                .is_some_and(|entries| has_hash(&entries, &places.project, &hash));
            if trusted {
                hooks.project_hash = Some(hash);
                hooks.defs.extend(project_defs);
            } else {
                hooks.skipped_untrusted = project_defs.len();
                hooks.untrusted_defs = project_defs;
                if places.user.as_deref().is_some_and(|user| is_trusted(user, &places.project)) {
                    hooks.problems.push("project hooks changed since you trusted them; review with /hooks and run /hooks trust again".into());
                }
            }
        }
        let settings_trusted = places.user.as_deref().is_some_and(|user| claude_settings_trusted(user, &places.project));
        if !settings_trusted {
            hooks.untrusted_settings = claude_settings_preview(&places.project);
            hooks.settings_changed = !hooks.untrusted_settings.is_empty()
                && places.user.as_deref().is_some_and(|user| is_trusted(user, &places.project));
        }
        hooks
    }

    /// What the project's `.claude/settings.json` holds while it is not trusted (so not loaded).
    pub fn untrusted_settings(&self) -> &[(String, String)] {
        &self.untrusted_settings
    }

    pub fn defs(&self) -> &[HookDef] {
        &self.defs
    }

    pub fn untrusted_defs(&self) -> &[HookDef] {
        &self.untrusted_defs
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Run every hook for `event` in turn (blocking).
    pub fn run(&self, event: Event, tool: Option<&str>, summary: Option<&str>) -> Outcome {
        let payload = Payload { event, tool, summary, project: &self.dir };
        let Ok(input) = serde_json::to_string(&payload) else { return Outcome::default() };
        let mut outcome = Outcome::default();
        for (index, def) in self.defs.iter().enumerate().filter(|(_, d)| d.matches(event, tool)) {
            if index >= self.project_start && !self.project_hash.as_ref().is_some_and(|hash| {
                std::fs::read(self.dir.join(".jev/hooks.json")).is_ok_and(|bytes| sha256::hex(&bytes) == *hash)
            }) {
                outcome.warnings.push("project hooks changed since you trusted them; review with /hooks and run /hooks trust again".into());
                break;
            }
            let timeout = Duration::from_secs(def.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
            match run_command(&def.run, &self.dir, &input, event, tool, timeout) {
                Ran::Exited { code: 0, .. } => {}
                Ran::Exited { code: 2, stderr } if event == Event::PreTool => {
                    let said = stderr.trim();
                    outcome.refused = Some(if said.is_empty() { format!("refused by the hook `{}`", clip(&def.run)) } else { said.to_string() });
                    return outcome;
                }
                Ran::Exited { code, .. } => outcome.warnings.push(format!("hook `{}` exited with status {code}", clip(&def.run))),
                Ran::TimedOut => outcome.warnings.push(format!("hook `{}` took longer than {}s and was stopped", clip(&def.run), timeout.as_secs())),
                Ran::NotStarted(why) => outcome.warnings.push(format!("hook `{}` did not start: {why}", clip(&def.run))),
            }
        }
        outcome
    }
}
