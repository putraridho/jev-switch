//! The trust, git, team and insight commands (`/commit`, `/pr`, `/profile`, `/team`,
//! `/audit`, `/history`, `/bench`, `/replay`) and the hooks the turn loop calls for them
//! (audit lines, the in-flight marker, shared team rules).

use super::{Flow, Io, Repl};
use crate::audit;
use crate::extcmds::ExtCmd;
use crate::mode::Mode;
use crate::profile::{self, Profile};
use crate::routing::Pin;
use crate::sessionlog::TurnRecord;
use crate::team::{self, Team};
use crate::{bench, gitflow, inflight, replay};

impl Repl {
    pub(super) async fn trust(&mut self, cmd: ExtCmd, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        match cmd {
            ExtCmd::Commit(hint) => {
                let prompt = gitflow::commit_prompt(hint.as_deref());
                return self.canned(&prompt, "/commit", None, io).await;
            }
            ExtCmd::Pr(base) => {
                let branch = gitflow::current_branch(self.engine.dir());
                let task = self.engine.sessions.first_prompt();
                let prompt = gitflow::pr_prompt(base.as_deref(), branch.as_deref(), task.as_deref());
                return self.canned(&prompt, "/pr", None, io).await;
            }
            ExtCmd::Profile(None) => {
                let cap = self.team().ok().and_then(|t| t.max_profile);
                self.page_out(&profile::list_text(Profile::of_mode(self.engine.mode()), cap));
            }
            ExtCmd::Profile(Some(wanted)) => {
                let cap = self.team().ok().and_then(|t| t.max_profile);
                let applied = wanted.capped_by(cap);
                if applied != wanted {
                    self.console.out(&format!("{} {} is above this repo's limit ({applied}); using {applied}", style.yellow("⚠"), wanted));
                }
                self.engine.set_mode(applied.mode());
                self.remember_mode();
                self.audit_event(audit::Event::Profile { name: applied.name().to_string() });
                self.console.out(&format!("  profile: {applied} ({})", applied.about()));
            }
            ExtCmd::Team => match self.team() {
                Ok(t) => self.page_out(&t.text()),
                Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
            },
            ExtCmd::Audit(n) => self.page_out(&audit::audit_text(&self.audit_entries(), n)),
            ExtCmd::History => self.page_out(&audit::history_text(&audit::history(&self.audit_entries()))),
            ExtCmd::Bench => {
                let ranks = bench::rank(&self.audit_entries());
                self.page_out(&bench::bench_text(&ranks, self.engine.dir()));
            }
            ExtCmd::Replay(n) => self.replay(n),
            ExtCmd::Invalid(message) => self.console.out(&format!("{} {message}", style.yellow("⚠"))),
        }
        Flow::Continue
    }

    fn replay(&mut self, n: usize) {
        let style = self.out_style();
        let all = match self.engine.resumable() {
            Ok(all) => all,
            Err(e) => return self.resume_failed(&e),
        };
        let Some(entry) = n.checked_sub(1).and_then(|i| all.get(i)) else {
            let text = format!("There is no session {n}; there are {}.", all.len());
            return self.console.out(&format!("  {}", style.dim(&text)));
        };
        match crate::handoff::load_path(self.engine.dir(), &entry.path) {
            Ok(saved) => match replay::replay_text(&entry.title, saved.sessions.records()) {
                Some(text) => self.page_out(&text),
                None => self.console.out(&format!("  {}", style.dim("That session has no turns."))),
            },
            Err(e) => self.resume_failed(&e),
        }
    }

    // ------------------------------------------------------------------ team

    fn team(&self) -> Result<Team, team::TeamError> {
        Team::load(self.engine.dir())
    }

    /// The mode, lowered to what the repo's shared file allows (and said so).
    pub(super) fn within_team_limit(&self, mode: Mode) -> Mode {
        let cap = match self.team() {
            Ok(t) => t.max_profile,
            // A shared file that cannot be read must not quietly lift its own limit.
            Err(e) => {
                self.console.out(&format!("  {}", self.out_style().yellow(&format!("{e}; the repo's limit could not be applied"))));
                None
            }
        };
        let allowed = Profile::of_mode(mode).capped_by(cap).mode();
        if allowed != mode {
            self.console.out(&format!("  {}", self.out_style().yellow(&format!("{mode} mode is above this repo's limit ({}); using {allowed}", team::FILE))));
        }
        allowed
    }

    /// Pin the model a shared rule names for `prompt`, only while nothing is pinned by hand.
    /// True when it pinned (the caller puts the pin back after the turn).
    pub(super) fn apply_team_route(&mut self, prompt: &str) -> bool {
        if *self.engine.pin() != Pin::Auto {
            return false;
        }
        let Ok(team) = self.team() else { return false };
        let Some(pin) = team.pin_for(prompt).cloned() else { return false };
        self.console.err(&format!("  {}", self.console.err_style().dim(&format!("{}: routed to {pin}", team::FILE))));
        self.engine.set_pin(pin);
        true
    }

    // ------------------------------------------------------ audit and recovery

    fn audit_path(&self) -> Option<std::path::PathBuf> {
        self.state_dir.as_deref().map(audit::path_in)
    }

    /// The log's entries for this project (the file is shared by every project).
    fn audit_entries(&self) -> Vec<audit::Entry> {
        let project = self.engine.dir().display().to_string();
        let mut all = self.audit_path().map(|p| audit::read(&p)).unwrap_or_default();
        all.retain(|e| e.project == project);
        all
    }

    /// Hold the mode a session starts in (`--mode`, the saved preference, `CODE_PERMISSION_MODE`)
    /// to the repo's limit, the same way a mode chosen later is.
    pub fn enforce_team_limit(&mut self) {
        let mode = self.engine.mode();
        let allowed = self.within_team_limit(mode);
        if allowed != mode {
            self.engine.set_mode(allowed);
        }
    }

    /// A mode picked by a key (not a command): held to the repo's limit, kept as the
    /// preference, and the mode that applies is returned.
    pub fn adopt_mode(&mut self, mode: Mode) -> Mode {
        let mode = self.within_team_limit(mode);
        self.engine.set_mode(mode);
        self.remember_mode();
        mode
    }

    /// A turn that was abandoned (second Ctrl-C) still happened: say so in the log.
    pub(super) fn audit_aborted(&self, prompt: &str, elapsed: std::time::Duration) {
        let record = TurnRecord {
            prompt: prompt.to_string(),
            answer: String::new(),
            status: crate::turn::TurnStatus::Cancelled,
            model: String::new(),
            route: None,
            usage: None,
            elapsed_ms: elapsed.as_millis() as u64,
            ended_ms: self.now(),
        };
        self.audit_turn(&record);
    }

    pub(super) fn audit_event(&self, event: audit::Event) {
        if let Some(path) = self.audit_path() {
            let entry = audit::Entry { ts_ms: self.now(), project: self.engine.dir().display().to_string(), event };
            audit::append(&path, &entry);
        }
    }

    pub(super) fn audit_turn(&self, record: &TurnRecord) {
        if let Some(path) = self.audit_path() {
            audit::append(&path, &audit::turn_entry(self.now(), &self.engine.dir().display().to_string(), record));
        }
    }

    pub(super) fn mark_turn_started(&self, prompt: &str) {
        if let Some(dir) = &self.state_dir {
            inflight::begin(dir, self.engine.dir(), prompt, self.now());
        }
    }

    pub(super) fn mark_turn_ended(&self) {
        if let Some(dir) = &self.state_dir {
            inflight::end(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{rig, run, ANSWER};

    #[tokio::test]
    async fn profile_command_sets_the_mode_and_a_team_file_caps_it() {
        let mut r = rig("profile", ANSWER);
        run(&mut r, "/profile").await;
        assert!(r.out.text().contains("Permission profiles"), "{}", r.out.text());
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"max_profile":"careful"}"#).unwrap();
        run(&mut r, "/profile trusted").await;
        assert_eq!(r.repl.engine.mode(), crate::mode::Mode::Ask);
        assert!(r.out.text().contains("above this repo's limit"), "{}", r.out.text());
        run(&mut r, "/mode auto").await;
        assert_eq!(r.repl.engine.mode(), crate::mode::Mode::Ask);
    }

    #[tokio::test]
    async fn a_mode_picked_by_key_is_held_to_the_team_limit() {
        let mut r = rig("keymode", ANSWER);
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"max_profile":"careful"}"#).unwrap();
        let applied = r.repl.adopt_mode(crate::mode::Mode::Auto);
        assert_eq!(applied, crate::mode::Mode::Ask);
        assert_eq!(r.repl.engine.mode(), crate::mode::Mode::Ask);
    }

    fn state_in(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jev-state-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn insight_commands_read_only_this_projects_entries() {
        let mut r = rig("scoped", ANSWER);
        let state = state_in("scoped");
        r.repl.state_dir = Some(state.clone());
        let entry = |project: &str, prompt: &str| crate::audit::Entry {
            ts_ms: 1000,
            project: project.into(),
            event: crate::audit::Event::Turn { model: "m".into(), outcome: crate::audit::Outcome::Answered, elapsed_ms: 1, in_tokens: 1, out_tokens: 1, prompt: prompt.into() },
        };
        let path = crate::audit::path_in(&state);
        crate::audit::append(&path, &entry("/somewhere/else", "other project prompt"));
        crate::audit::append(&path, &entry(&r.dir.display().to_string(), "this project prompt"));
        run(&mut r, "/audit").await;
        let text = r.out.text();
        assert!(text.contains("this project prompt") && !text.contains("other project prompt"), "{text}");
    }

    #[tokio::test]
    async fn an_abandoned_turn_is_in_the_audit_log() {
        let mut r = rig("abandoned", ANSWER);
        let state = state_in("abandoned");
        r.repl.state_dir = Some(state.clone());
        r.repl.audit_aborted("long task", std::time::Duration::from_millis(1500));
        let entries = crate::audit::read(&crate::audit::path_in(&state));
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0].event, crate::audit::Event::Turn { outcome: crate::audit::Outcome::Cancelled, elapsed_ms: 1500, prompt, .. } if prompt == "long task"), "{entries:?}");
    }

    #[tokio::test]
    async fn the_crash_marker_names_the_prompt_not_the_waiting_notes() {
        let state = state_in("marker");
        let script = format!("cp {}/inflight-*.json seen.json; printf 'FIN:ok\\n'", state.display());
        let mut r = rig("marker", &script);
        r.repl.state_dir = Some(state.clone());
        r.repl.carry.push("summary of earlier work".into());
        run(&mut r, "my actual question").await;
        let seen = std::fs::read_to_string(r.dir.join("seen.json")).expect("the marker existed during the turn");
        assert!(seen.contains("my actual question") && !seen.contains("summary of earlier"), "{seen}");
    }

    #[tokio::test]
    async fn team_command_shows_the_rules() {
        let mut r = rig("team", ANSWER);
        run(&mut r, "/team").await;
        assert!(r.out.text().contains("No shared rules"), "{}", r.out.text());
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"routing":[{"contains":"docs","pin":"codex"}]}"#).unwrap();
        run(&mut r, "/team").await;
        assert!(r.out.text().contains("\"docs\" go to codex"), "{}", r.out.text());
    }

    #[tokio::test]
    async fn insight_commands_are_calm_when_nothing_is_recorded() {
        let mut r = rig("insight", ANSWER);
        for line in ["/audit", "/history", "/bench", "/replay"] {
            run(&mut r, line).await;
        }
        let text = r.out.text();
        assert!(text.contains("audit log is empty") || text.contains("No usage history") || text.contains("No ranking"), "{text}");
    }
}
