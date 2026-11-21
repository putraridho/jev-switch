//! The project and setup commands: `/memory`, `/config`, `/update`, `/terminal-setup`.

use super::{clip, tilde};
use std::path::{Path, PathBuf};

use jev_approval::domain::settings_document;
use crate::repl::session::{Io, Repl};
use jev_shared::infrastructure::env_file as envfile;
use crate::app::host;
use jev_shared::infrastructure::jsonl_log as log;
use crate::tty::terminal_setup as termsetup;
use crate::tty::terminal_setup::{Installed, TermEnv};
use crate::render as uifmt;
use crate::render::{ConfigRow, TerminalSetupFacts};

/// The instruction files the CLIs read, in the order `/memory` opens them.
const INSTRUCTION_FILES: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

impl Repl {

    /// `/memory`: open the first instructions file that exists (else a new AGENTS.md).
    pub(in crate::repl) fn memory(&mut self) {
        let style = self.out_style();
        let dir = self.engine.dir().to_path_buf();
        let existing: Vec<&str> = INSTRUCTION_FILES.iter().copied().filter(|f| dir.join(f).is_file()).collect();
        let file = existing.first().copied().unwrap_or("AGENTS.md");
        self.console.out(&format!(
            "  {}",
            style.dim("Codex reads AGENTS.md, Claude CLAUDE.md, Gemini GEMINI.md; /init writes AGENTS.md and points the others at it.")
        ));
        match self.console.with_terminal(|| host::editor::edit_file(&dir.join(file))) {
            Ok(()) if dir.join(file).is_file() => self.console.out(&format!("{} {file} saved; every model reads it from the next turn", style.green("✓"))),
            Ok(()) => self.console.out(&format!("  {}", style.dim(&format!("{file} was not created.")))),
            Err(e) => self.console.out(&format!("{} {e}", style.yellow("⚠"))),
        }
    }

    // ---------------------------------------------------------------- setup

    /// `/config`.
    pub(in crate::repl) fn config_text(&self) -> String {
        let row = |name: &str, value: String, change: &str| ConfigRow { name: name.into(), value, change: change.into() };
        let on_off = |on: bool| if on { "on" } else { "off" }.to_string();
        let session = vec![
            row("mode", self.engine.mode().to_string(), "/mode, Shift-Tab"),
            row("model", self.engine.pin().to_string(), "/model"),
            row("routing", on_off(self.engine.enabled()), "/provider auto|off"),
            row("orchestrate", on_off(self.engine.orchestrate()), "/orchestrate on|off"),
        ];
        let shown = |name: &str| envfile::lookup(name).map(|v| clip(&v, 36)).unwrap_or_else(|| "(default)".into());
        let set = |name: &str| if envfile::lookup(name).is_some_and(|v| !v.is_empty()) { "set" } else { "not set" }.to_string();
        let env = vec![
            row("CODE_PERMISSION_MODE", shown("CODE_PERMISSION_MODE"), "the mode a session starts in"),
            row("JEV_PROVIDERS", shown("JEV_PROVIDERS"), "off starts with routing off; /provider is kept"),
            row("JEV_ORCHESTRATE", shown("JEV_ORCHESTRATE"), "on|off at start; /orchestrate is kept"),
            row("CODE_ALLOWED_TOOLS", shown("CODE_ALLOWED_TOOLS"), "tools allowed without asking"),
            row("DOWNGRADE_AFTER", shown("DOWNGRADE_AFTER"), "turns before a cheaper tier is tried"),
            row("CODE_STRONG_THRESHOLD", shown("CODE_STRONG_THRESHOLD"), "when routing is off: hard-task cut-off"),
            row("JEV_CHAINS", set("JEV_CHAINS"), "model order per task (JSON)"),
            row("PAGER", shown("PAGER"), "long output and /diff"),
            row("EDITOR", shown("VISUAL").replace("(default)", &shown("EDITOR")), "/edit and /memory"),
            row("JEV_API_KEY", set("JEV_API_KEY"), "Jev, which judges each task"),
            row("DEEPSEEK_API_KEY", set("DEEPSEEK_API_KEY"), "DeepSeek through Claude Code"),
        ];
        let home = self.home.clone();
        let env_file = envfile::env_file_path(std::env::var("JEV_ROUTER_ENV").ok(), home.clone());
        let state = log::state_dir();
        let mut files: Vec<(String, String)> = Vec::new();
        let tilde = |p: &Path| tilde(p, home.as_deref());
        if let Some(f) = env_file {
            files.push(("settings".into(), tilde(&f)));
        }
        files.push(("permissions".into(), tilde(&self.engine.dir().join(settings_document::LOCAL_SETTINGS))));
        if let Some(s) = &state {
            files.push(("history".into(), tilde(&crate::input::line_editor::history::history_path(s))));
            files.push(("logs".into(), tilde(s)));
        }
        if let Some(p) = self.engine.session_path() {
            files.push(("session".into(), tilde(p)));
        }
        uifmt::config_text(&session, &env, &files, self.out_style())
    }

    /// `/update`: fetch the checkout's upstream, show what is new, and after a yes fast-forward
    /// and reinstall. The running session keeps the old program until it is restarted.
    pub(in crate::repl) fn update(&mut self, io: &mut Io<'_>) {
        use crate::app::self_update as selfupdate;
        use crate::app::self_update::Plan;
        let style = self.out_style();
        let dir = match selfupdate::source_dir() {
            Ok(dir) => dir,
            Err(why) => {
                self.console.out(&format!("{} cannot update: {why}", style.yellow("⚠")));
                return;
            }
        };
        self.console.out(&style.dim(&format!("  checking {} for a new version...", tilde(&dir, self.home.as_deref()))));
        let plan = match selfupdate::check(&dir) {
            Ok(facts) => selfupdate::plan(&facts, selfupdate::running_version()),
            Err(why) => Plan::Refused(why),
        };
        let lines = selfupdate::plan_text(&plan);
        let pull = match &plan {
            Plan::UpToDate { .. } => {
                self.console.out(&format!("{} {}", style.green("✓"), lines.join("\n")));
                return;
            }
            Plan::Refused(_) => {
                self.console.out(&format!("{} {}", style.yellow("⚠"), lines.join("\n  ")));
                return;
            }
            Plan::Update { pull, .. } => *pull,
        };
        let mut shown = lines.into_iter();
        if let Some(head) = shown.next() {
            self.console.out(&format!("  {}", style.bold(&head)));
        }
        for line in shown {
            self.console.out(&format!("  {}", style.dim(&line)));
        }
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!("{} Not updated: asking needs a terminal. Run `jev-code update` in one.", style.yellow("⚠")));
            return;
        };
        let question = if pull { "Pull it and reinstall jev-code now? [y/N] " } else { "Reinstall jev-code from the checkout now? [y/N] " };
        if !confirm.confirm(question) {
            self.console.out(&format!("  {}", style.dim("Left as it is.")));
            return;
        }
        let console = &self.console;
        match selfupdate::apply(&dir, pull, &mut |line| console.out(&format!("  {}", style.dim(line)))) {
            Ok(version) => self.console.out(&format!("{} jev-code {version} is installed. Restart jev-code to use it (this session keeps running the old one).", style.green("✓"))),
            Err(why) => self.console.out(&format!("{} {why}", style.yellow("⚠"))),
        }
    }

    /// `/terminal-setup [install]`.
    pub(in crate::repl) fn terminal_setup(&mut self, install: bool, io: &mut Io<'_>) {
        let style = self.out_style();
        let terminal = termsetup::detect(&TermEnv::from_process());
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| self.home.as_ref().map(|h| Path::new(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        let file = terminal.map(|t| t.config_file(&config_home));
        let facts = TerminalSetupFacts {
            terminal: terminal.map(|t| t.name().to_string()),
            file: file.as_deref().map(|f| tilde(f, self.home.as_deref())).unwrap_or_default(),
            snippet: terminal.map(|t| t.snippet().to_string()).unwrap_or_default(),
            reload: terminal.map(|t| t.reload_hint().to_string()).unwrap_or_default(),
            can_install: terminal.is_some_and(|t| t.can_append()),
        };
        if !install {
            self.page_out(&uifmt::terminal_setup_text(&facts, style));
            return;
        }
        let (Some(terminal), Some(file)) = (terminal, file) else {
            self.page_out(&uifmt::terminal_setup_text(&facts, style));
            return;
        };
        if !terminal.can_append() {
            self.console.out(&format!("{} {}'s config needs the lines placed by hand:", style.yellow("⚠"), terminal.name()));
            self.page_out(&uifmt::terminal_setup_text(&facts, style));
            return;
        }
        let Some(confirm) = io.confirm.as_mut() else {
            self.console.out(&format!("{} Not changed: asking needs a terminal.", style.yellow("⚠")));
            return;
        };
        if !confirm.confirm(&format!("Add Shift-Enter to {}? [y/N] ", facts.file)) {
            self.console.out(&format!("  {}", style.dim("Left as it is.")));
            return;
        }
        match termsetup::install(terminal, &file) {
            Ok(Installed::Added { backup }) => {
                let kept = backup.map(|b| format!(" (the old one is {})", tilde(&b, self.home.as_deref()))).unwrap_or_default();
                self.console.out(&format!("{} added to {}{kept}. Then: {}.", style.green("✓"), facts.file, facts.reload));
            }
            Ok(Installed::AlreadyThere) => self.console.out(&format!("  {}", style.dim(&format!("{} has it already. {}.", facts.file, facts.reload)))),
            Err(e) => self.console.out(&format!("{} not changed: {e}", style.yellow("⚠"))),
        }
    }
}
