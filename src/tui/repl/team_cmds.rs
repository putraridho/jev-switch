//! The trust, git, team and insight commands (`/commit`, `/pr`, `/profile`, `/team`,
//! `/audit`, `/history`, `/bench`, `/replay`) and the hooks the turn loop calls for them
//! (audit lines, the in-flight marker, shared team rules).

use crate::tui::repl::session::{Flow, Io, Repl};
use crate::turn::infrastructure::audit;
use crate::tui::repl::extended_commands::ExtCmd;
use crate::approval::domain::mode::Mode;
use crate::approval::domain::profile;
use crate::approval::domain::profile::Profile;
use crate::routing::domain::decide::Pin;
use crate::turn::domain::session_log::TurnRecord;
use crate::turn::application::team_rules as team;
use crate::turn::application::team_rules::Team;
use crate::calibration::application::bench;
use crate::tui::repl::gitflow;
use crate::turn::infrastructure::inflight;
use crate::session::application::replay;

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
                let (cap, _) = self.cap_or_locked(true);
                self.page_out(&profile::list_text(Profile::of_mode(self.engine.mode()), cap));
            }
            ExtCmd::Profile(Some(wanted)) => {
                let (cap, readable) = self.cap_or_locked(true);
                if !readable {
                    // Held to plan, and nothing saved: the person's choice waits for the fix.
                    self.engine.set_mode(Profile::Locked.mode());
                    self.warn("fix it before changing the profile");
                    return Flow::Continue;
                }
                let applied = wanted.capped_by(cap);
                if applied != wanted {
                    self.warn(&format!("{wanted} is above this repo's limit ({applied}); using {applied}"));
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
        match crate::session::domain::catch_up::load_path(self.engine.dir(), &entry.path) {
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
        self.held_to_limit(mode).0
    }

    /// The mode held to the repo's limit, and whether the limit could be read. A limit that
    /// cannot be read must not lift itself: the mode is held to plan until the file is fixed.
    fn held_to_limit(&self, mode: Mode) -> (Mode, bool) {
        // Said once: either the file is unreadable, or the mode is above the limit it sets.
        let (cap, readable) = self.cap_or_locked(mode != Mode::Plan);
        let allowed = Profile::of_mode(mode).capped_by(cap).mode();
        if readable && allowed != mode {
            self.warn(&format!("{mode} mode is above this repo's limit ({}); using {allowed}", team::FILE));
        }
        (allowed, readable)
    }

    /// The repo's profile limit and whether it could be read; an unreadable one is the
    /// narrowest profile (warned about when `warn` is set).
    fn cap_or_locked(&self, warn: bool) -> (Option<Profile>, bool) {
        match Team::load_cap(self.engine.dir()) {
            Ok(cap) => (cap, true),
            Err(e) => {
                if warn {
                    self.warn(&format!("{e}; holding to plan mode until the limit can be read"));
                }
                (Some(Profile::Locked), false)
            }
        }
    }

    /// A warning on stderr: in -p mode stdout carries the answer alone.
    pub(super) fn warn(&self, text: &str) {
        self.console.err(&format!("  {}", self.console.err_style().yellow(text)));
    }


    /// Pin the model a shared rule names for `prompt`, only while nothing is pinned by hand.
    /// True when it pinned (the caller puts the pin back after the turn).
    pub(super) fn apply_team_route(&mut self, prompt: &str) -> bool {
        if *self.engine.pin() != Pin::Auto {
            return false;
        }
        let Ok(team) = self.team() else { return false };
        let Some(pin) = team.pin_for(prompt).cloned() else { return false };
        // The rule comes with the repository; the person's own routing rules (`avoid`,
        // `only`) win over it. The engine applies them to a team pin once the turn's task is
        // known and routes automatically if they exclude it; the route's reason says which.
        self.console.err(&format!("  {}", self.console.err_style().dim(&format!("{}: asks for {pin} (your routing rules still apply)", team::FILE))));
        self.engine.set_team_pin(pin);
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
        let (held, readable) = self.held_to_limit(mode);
        self.engine.set_mode(held);
        // Plan forced by a broken file is not what the person picked: keep the old preference
        // (but plan picked on purpose is kept).
        if readable || held == mode {
            self.remember_mode();
        }
        held
    }

    /// A turn that was abandoned (second Ctrl-C) still happened: say so in the log.
    pub(super) fn audit_aborted(&self, prompt: &str, elapsed: std::time::Duration) {
        let record = TurnRecord {
            prompt: prompt.to_string(),
            answer: String::new(),
            status: crate::turn::application::engine::TurnStatus::Cancelled,
            model: String::new(),
            route: None,
            usage: None,
            elapsed_ms: elapsed.as_millis() as u64,
            ended_ms: self.now(),
            tool_calls: 0,
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
    use crate::tui::repl::session::tests::{rig, run, ANSWER};

    #[tokio::test]
    async fn profile_command_sets_the_mode_and_a_team_file_caps_it() {
        let mut r = rig("profile", ANSWER);
        run(&mut r, "/profile").await;
        assert!(r.out.text().contains("Permission profiles"), "{}", r.out.text());
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"max_profile":"careful"}"#).unwrap();
        run(&mut r, "/profile trusted").await;
        assert_eq!(r.repl.engine.mode(), crate::approval::domain::mode::Mode::Ask);
        assert!(r.err.text().contains("above this repo's limit"), "{}", r.err.text());
        run(&mut r, "/mode auto").await;
        assert_eq!(r.repl.engine.mode(), crate::approval::domain::mode::Mode::Ask);
    }

    #[tokio::test]
    async fn an_unreadable_team_limit_holds_profile_and_keys_to_plan() {
        let mut r = rig("badteam", ANSWER);
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"max_profile":"careful",}"#).unwrap();
        run(&mut r, "/profile trusted").await;
        assert_eq!(r.repl.engine.mode(), crate::approval::domain::mode::Mode::Plan, "held to plan until the file is fixed");
        assert!(r.err.text().contains("fix it before changing the profile"), "{}", r.err.text());
        let seen = r.err.text().len();
        assert_eq!(r.repl.adopt_mode(crate::approval::domain::mode::Mode::Auto), crate::approval::domain::mode::Mode::Plan);
        let said = r.err.text()[seen..].to_string();
        assert!(said.matches("holding to plan mode").count() == 1 && !said.contains("above this repo's limit"), "one warning: {said}");
        // A routing rule this version cannot read leaves the limit itself in force.
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"routing":[{"contains":"x","pin":"nosuch:m"}],"max_profile":"careful"}"#).unwrap();
        assert_eq!(r.repl.adopt_mode(crate::approval::domain::mode::Mode::Auto), crate::approval::domain::mode::Mode::Ask);
    }

    #[tokio::test]
    async fn a_mode_picked_by_key_is_held_to_the_team_limit() {
        let mut r = rig("keymode", ANSWER);
        std::fs::create_dir_all(r.dir.join(".jev")).unwrap();
        std::fs::write(r.dir.join(".jev/team.json"), r#"{"max_profile":"careful"}"#).unwrap();
        let applied = r.repl.adopt_mode(crate::approval::domain::mode::Mode::Auto);
        assert_eq!(applied, crate::approval::domain::mode::Mode::Ask);
        assert_eq!(r.repl.engine.mode(), crate::approval::domain::mode::Mode::Ask);
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
        let entry = |project: &str, prompt: &str| crate::turn::infrastructure::audit::Entry {
            ts_ms: 1000,
            project: project.into(),
            event: crate::turn::infrastructure::audit::Event::Turn { model: "m".into(), outcome: crate::turn::infrastructure::audit::Outcome::Answered, elapsed_ms: 1, in_tokens: 1, out_tokens: 1, prompt: prompt.into(), cached_tokens: 0, context_tokens: 0, tool_calls: 0 },
        };
        let path = crate::turn::infrastructure::audit::path_in(&state);
        crate::turn::infrastructure::audit::append(&path, &entry("/somewhere/else", "other project prompt"));
        crate::turn::infrastructure::audit::append(&path, &entry(&r.dir.display().to_string(), "this project prompt"));
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
        let entries = crate::turn::infrastructure::audit::read(&crate::turn::infrastructure::audit::path_in(&state));
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0].event, crate::turn::infrastructure::audit::Event::Turn { outcome: crate::turn::infrastructure::audit::Outcome::Cancelled, elapsed_ms: 1500, prompt, .. } if prompt == "long task"), "{entries:?}");
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
