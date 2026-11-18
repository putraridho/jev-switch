//! `/project` (kind, map, checks with a fix loop, notes) and the context helpers that go
//! with it: project memory in front of a session's first message, and auto-compaction.
//! The rules live in [`crate::projectkind`], [`crate::repomap`], [`crate::projmemory`] and
//! [`crate::ctxintel`]; this maps them onto the screen.

use super::{Flow, Io, Repl};
use crate::ctxintel::{self, AutoCompact};
use crate::envfile;
use crate::host;
use crate::projcmd::ProjectCmd;
use crate::projectkind::{self, Check, Project};
use crate::projmemory::Memory;
use crate::repomap;

/// How many times `/project check ... fix` asks the model to fix and reruns.
pub(super) const FIX_ROUNDS: usize = 3;

/// Characters of repo map a `/project map` puts in front of the model.
const MAP_BUDGET: usize = 6_000;

/// What the model is told when a check failed and it is to fix it.
pub(super) fn fix_prompt(check: Check, command: &str, round: usize) -> String {
    format!(
        "The {check} check (`{command}`) failed; its output is above (attempt {round} of {FIX_ROUNDS}). Fix the cause in the code, \
without weakening or deleting the check. I will run it again when you are done."
    )
}

impl Repl {
    pub(super) async fn project(&mut self, cmd: ProjectCmd, io: &mut Io<'_>) -> Flow {
        match cmd {
            ProjectCmd::Overview => self.page_out(&self.project_overview()),
            ProjectCmd::Map(words) => self.repo_map(&words),
            ProjectCmd::Check { check, fix } => return self.run_check(check, fix, io).await,
            ProjectCmd::Note(text) => self.add_note(&text),
            ProjectCmd::Notes => self.page_out(&self.notes_text()),
            ProjectCmd::Forget(n) => self.forget_note(n),
        }
        Flow::Continue
    }

    fn memory_store(&self) -> Option<Memory> {
        self.engine.state_dir().map(|state| Memory::load(state, self.engine.dir()))
    }

    /// The project memory a session's first message starts with.
    pub(super) fn memory_note(&self) -> Option<String> {
        self.memory_store().and_then(|m| m.as_note())
    }

    fn project_overview(&self) -> String {
        let style = self.out_style();
        let found = projectkind::detect(self.engine.dir());
        let mut lines = vec![style.bold("Project")];
        if found.is_empty() {
            lines.push(format!("  {}", style.dim("No Cargo.toml, package.json, pyproject.toml or go.mod here, so no checks to run.")));
        }
        for p in &found {
            lines.push(format!("  {}", p.kind));
            for check in [Check::Test, Check::Lint, Check::Build] {
                if let Some(command) = p.command(check) {
                    lines.push(format!("    {}{}", style.dim(&format!("{check:<6}")), style.cyan(command)));
                }
            }
        }
        let notes = self.memory_store().map_or(0, |m| m.notes().len());
        lines.push(format!("  {}", style.dim(&format!("{notes} note(s) · /project map · /project check [test|lint|build] [fix] · /project note <text>"))));
        lines.join("\n")
    }

    fn repo_map(&mut self, words: &[String]) {
        let style = self.out_style();
        let map = repomap::build(self.engine.dir());
        if map.is_empty() {
            return self.console.out(&format!("  {}", style.dim("No source files with symbols found (Rust, Python, JS/TS and Go are read).")));
        }
        let text = repomap::render(&map, words, MAP_BUDGET);
        self.carry.push(repomap::note(&text));
        self.console.out(&text);
        self.console.out(&format!("  {}", style.dim(&format!("{} files mapped · the map goes with your next message", map.len()))));
    }

    /// The command for `check`, or why there is none.
    fn check_command(&self, check: Check) -> Result<(Project, String), String> {
        let found = projectkind::detect(self.engine.dir());
        if found.is_empty() {
            return Err("This folder is not a Cargo, npm, Python or Go project.".into());
        }
        found
            .into_iter()
            .find_map(|p| p.command(check).map(str::to_string).map(|c| (p, c)))
            .ok_or_else(|| format!("No {check} command known for this project; /project lists what is."))
    }

    /// Run the check; with `fix`, ask the model to repair each failure and rerun, up to
    /// [`FIX_ROUNDS`] times.
    async fn run_check(&mut self, check: Check, fix: bool, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        match self.check_command(check) {
            Ok((_, command)) => self.run_check_command(check, &command, fix, io).await,
            Err(why) => {
                self.console.out(&format!("{} {why}", style.yellow("⚠")));
                Flow::Continue
            }
        }
    }

    pub(super) async fn run_check_command(&mut self, check: Check, command: &str, fix: bool, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        for round in 1..=FIX_ROUNDS {
            self.console.out(&format!("  {} {}", style.dim("running"), style.cyan(&command)));
            let console = self.console.clone();
            let run = host::run_shell(&command, self.engine.dir(), &mut |line| console.out(&format!("  {}", style.dim(line))));
            if run.failed.is_none() && run.code == Some(0) {
                self.console.out(&format!("{} {check} passed", style.green("✓")));
                return Flow::Continue;
            }
            self.carry.push(host::shell_note(&run));
            if !fix {
                self.console.out(&format!("{} {check} failed {}", style.yellow("⚠"), style.dim("· the output goes with your next message; /project check fix repairs and reruns")));
                return Flow::Continue;
            }
            if round == FIX_ROUNDS {
                self.console.out(&format!("{} {check} still fails after {FIX_ROUNDS} fix attempts; the output goes with your next message", style.yellow("⚠")));
                return Flow::Continue;
            }
            let answered = self.prompt(&fix_prompt(check, &command, round), io).await;
            if !matches!(answered, Flow::Continue) || self.carry_waiting() {
                // Interrupted, or the model could not answer (its notes are still waiting).
                return answered;
            }
        }
        Flow::Continue
    }

    /// Whether notes are still waiting: a turn that answered clears them.
    fn carry_waiting(&self) -> bool {
        !self.carry.is_empty()
    }

    fn add_note(&mut self, text: &str) {
        let style = self.out_style();
        let Some(mut memory) = self.memory_store() else {
            return self.console.out(&format!("  {}", style.dim("Saving is switched off, so there is nowhere to keep notes.")));
        };
        match memory.add(text) {
            Ok(()) => self.console.out(&format!("{} noted ({} for this project): every new session starts with it", style.green("✓"), memory.notes().len())),
            Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        }
    }

    fn notes_text(&self) -> String {
        let style = self.out_style();
        let notes = self.memory_store().map(|m| m.notes().to_vec()).unwrap_or_default();
        if notes.is_empty() {
            return format!("  {}", style.dim("No notes. /project note <text> keeps one that every session of this project starts with."));
        }
        let mut lines = vec![style.bold("Project notes")];
        lines.extend(notes.iter().enumerate().map(|(i, n)| format!("  {} {n}", style.dim(&format!("{:>2}.", i + 1)))));
        lines.join("\n")
    }

    fn forget_note(&mut self, n: usize) {
        let style = self.out_style();
        let Some(mut memory) = self.memory_store() else {
            return self.console.out(&format!("  {}", style.dim("Saving is switched off, so there are no notes.")));
        };
        match memory.forget(n) {
            Ok(gone) => self.console.out(&format!("{} forgot \u{201c}{gone}\u{201d}", style.green("✓"))),
            Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        }
    }

    // ------------------------------------------------------------- context

    pub(super) fn auto_compact(&self) -> AutoCompact {
        AutoCompact::parse(envfile::lookup("JEV_AUTOCOMPACT").as_deref())
    }

    /// Before a message: compact first when the window is nearly full. `Some` when the
    /// person interrupted it and the message must not go.
    pub(super) async fn auto_compact_first(&mut self, io: &mut Io<'_>) -> Option<Flow> {
        if !self.auto_compact().due(self.context_percent(), self.log.turns().len()) {
            return None;
        }
        let style = self.out_style();
        self.console.out(&format!("  {}", style.dim(&format!("context is {}% full: compacting before this message", self.context_percent().unwrap_or(100)))));
        match self.compact(None, io).await {
            Flow::Aborted => Some(Flow::Aborted),
            _ => None,
        }
    }

    /// The "what fills it" rows of `/context`.
    pub(super) fn context_breakdown(&self) -> String {
        let memory = self.memory_note();
        let b = ctxintel::breakdown(self.log.turns(), &self.carry, memory.as_deref());
        ctxintel::breakdown_text(&b, self.auto_compact(), self.out_style())
    }
}
