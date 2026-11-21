//! Composing: carried notes, pasted attachments, `!command`, `#note`, `/edit` and canned prompts.


use crate::tui::repl::session::{Flow, Io, Repl};
use crate::turn::application::attach;
use crate::tui::app::host;
use crate::approval::domain::mode::Mode;

impl Repl {

    /// `text` with the waiting notes in front of it.
    pub(in crate::tui::repl) fn with_carry(&self, text: &str) -> String {
        // Project memory starts a session: it goes with the first message.
        let memory = if self.log.is_empty() { self.memory_note() } else { None };
        let notes: Vec<&str> = memory.iter().chain(self.carry.iter()).map(String::as_str).collect();
        if notes.is_empty() {
            return text.to_string();
        }
        format!("{}\n\n---\n\n{text}", notes.join("\n\n"))
    }

    /// The pasted paths of what the person typed (never of carried notes): images go to
    /// the engine for the next turn and are named on screen; the text comes back with them
    /// replaced by `[image: name]`.
    pub(in crate::tui::repl) fn take_attachments(&mut self, typed: &str) -> String {
        let found = attach::extract(typed, self.engine.dir());
        let style = self.console.err_style();
        for image in &found.images {
            let name = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.console.err(&format!("  {} {}", style.cyan("⎘"), style.dim(&format!("image: {name}"))));
        }
        self.engine.attach_images(found.images);
        found.text
    }

    /// `!command`: run it, show its output as it comes, keep it for the next message.
    pub(in crate::tui::repl) fn shell(&mut self, command: &str) {
        let style = self.out_style();
        let console = self.console.clone();
        let run = host::shell::run_shell(command, self.engine.dir(), &mut |line| console.out(&format!("  {}", style.dim(line))));
        let status = match (&run.failed, run.code) {
            (Some(why), _) => format!("{} could not start: {why}", style.red("✗")),
            (None, Some(0)) => format!("{} exit 0", style.green("✓")),
            (None, Some(code)) => format!("{} exit {code}", style.yellow("⚠")),
            (None, None) => format!("{} stopped by a signal", style.yellow("■")),
        };
        self.carry.push(host::shell::shell_note(&run));
        self.console.out(&format!("  {status} {}", style.dim("· the output goes with your next message")));
        self.console.out("");
    }

    /// `#note`: append it to the project's AGENTS.md.
    pub(in crate::tui::repl) fn note(&mut self, note: &str) {
        let style = self.out_style();
        let path = self.engine.dir().join("AGENTS.md");
        let line = match crate::tui::input::shortcuts::append_note(&path, note) {
            Ok(()) => format!("{} noted in AGENTS.md", style.green("✓")),
            Err(e) => format!("{} could not write AGENTS.md: {e}", style.yellow("⚠")),
        };
        self.console.out(&line);
        self.console.out("");
    }

    /// `/edit`: write the message in the editor, then send it.
    pub(in crate::tui::repl) async fn edit(&mut self, io: &mut Io<'_>) -> Flow {
        let style = self.out_style();
        match self.console.with_terminal(|| host::editor::compose(self.state_dir.as_deref())) {
            Ok(Some(text)) => {
                let first = text.lines().next().unwrap_or_default();
                let more = text.lines().count().saturating_sub(1);
                let tail = if more > 0 { format!(" (+{more} lines)") } else { String::new() };
                self.console.out(&format!("{} {first}{}", style.cyan("›"), style.dim(&tail)));
                self.prompt(&text, io).await
            }
            Ok(None) => {
                self.console.out(&format!("  {}", style.dim("Nothing written, nothing sent.")));
                Flow::Continue
            }
            Err(e) => {
                self.console.out(&format!("{} {e}", style.yellow("⚠")));
                Flow::Continue
            }
        }
    }

    /// A prompt the command writes for the person, run under `mode` when given (then put
    /// back), and kept in the log under the command's name.
    pub(in crate::tui::repl) async fn canned(&mut self, prompt: &str, label: &str, mode: Option<Mode>, io: &mut Io<'_>) -> Flow {
        let before = self.engine.mode();
        if let Some(m) = mode {
            self.engine.set_mode(m);
            self.console.err(&format!("  {}", self.console.err_style().dim(&format!("{label} runs in {m} mode: it is asked not to edit, and Claude and Codex cannot"))));
        }
        let sent = self.with_carry(prompt);
        let flow = self.run_recorded(&sent, label, io).await;
        self.engine.set_mode(before);
        flow
    }
}
