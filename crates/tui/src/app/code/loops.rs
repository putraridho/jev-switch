//! Owns the line loops of a session: the line editor on a terminal and the plain pipe.

use std::path::Path;
use std::time::Instant;
use crate::command as commands;
use crate::console::terminal_ui::Console;
use crate::input::line_editor as lineedit;
use crate::input::line_editor::interrupt::InterruptGuard;
use crate::input::line_editor::Input;
use jev_shared::infrastructure::jsonl_log as log;
use jev_approval::domain::mode;
use crate::repl::session::turn_race::{Interrupt, SigInt};
use crate::repl::session::{Confirm, Flow, Io, Repl};
use crate::views::status_facts as views;
use super::session::settle_late_probes;
use super::term::Term;

/// Lets `/undo` ask its question through the line editor.
pub(super) struct EditorConfirm<'a>(&'a mut lineedit::LineEditor);

impl Confirm for EditorConfirm<'_> {
    fn confirm(&mut self, question: &str) -> bool {
        tokio::task::block_in_place(|| lineedit::confirm(self.0, question))
    }
}

/// The interactive loop: the line editor, one message or command at a time.
pub(super) async fn edit_loop(repl: &mut Repl, term: &Term, console: &Console, state_dir: Option<&Path>, sigint: &mut SigInt) -> i32 {
    let history = state_dir.map(lineedit::history::history_path);
    // Shift-Tab changes this while a prompt is open; it is read back after every line.
    let shared_mode = mode::SharedMode::new(repl.engine.mode());
    let mut editor = match lineedit::build_editor(term.out_style, history.as_deref(), shared_mode.clone()) {
        Ok(e) => e,
        Err(e) => {
            console.err(&format!("jev-code: cannot start the line editor: {}", lineedit::describe(&e)));
            return pipe_loop(repl, sigint).await;
        }
    };
    if let Some(helper) = editor.helper_mut() {
        helper.set_project(repl.engine.dir());
    }
    let mut guard = InterruptGuard::default();
    loop {
        settle_late_probes(repl);
        shared_mode.set(repl.engine.mode());
        let label = views::prompt::prompt_label(repl.engine.pin(), repl.engine.enabled());
        let plain = views::prompt::plain_prompt(label.as_deref());
        let styled = views::prompt::styled_prompt(label.as_deref(), term.out_style);
        let ctx = views::providers::complete_ctx(&repl.engine, log::now_ms());
        // A message left unfinished when the last turn ended comes back into the prompt.
        let draft = console.take_draft();
        let input = tokio::task::block_in_place(|| lineedit::read(&mut editor, &plain, &styled, ctx, &draft));
        // A mode picked with Shift-Tab holds even when the line was thrown away.
        if shared_mode.get() != repl.engine.mode() {
            let applied = repl.adopt_mode(shared_mode.get());
            shared_mode.set(applied);
            console.out(&format!("  mode: {}", views::prompt::mode_line(applied)));
        }
        match input {
            Input::Line(line) => {
                settle_late_probes(repl);
                guard.reset();
                lineedit::history::remember(&mut editor, &line, history.as_deref());
                let mut flow = run_line(repl, &mut editor, sigint, &line).await;
                // Messages typed during the turn run next, in order, until one stops the loop.
                while flow == Flow::Continue {
                    let Some(next) = console.take_queued() else { break };
                    console.out(&format!("{} {}", term.out_style.cyan("›"), next.replace('\n', "\n  ")));
                    lineedit::history::remember(&mut editor, &next, history.as_deref());
                    flow = run_line(repl, &mut editor, sigint, &next).await;
                }
                match flow {
                    Flow::Continue => {}
                    Flow::Quit => return 0,
                    Flow::Aborted => return 130,
                }
            }
            Input::Interrupted => match guard.on_interrupt(Instant::now()) {
                lineedit::interrupt::Interrupt::Warn => console.err(&format!("\n{}", term.err_style.dim(lineedit::interrupt::EXIT_HINT))),
                lineedit::interrupt::Interrupt::Exit => return 0,
            },
            Input::Eof => {
                console.out("");
                return 0;
            }
            Input::Failed(message) => {
                console.err(&format!("jev-code: {message}"));
                return 1;
            }
        }
    }
}

/// One line from the person (typed, or queued during a turn), with `/undo` able to ask.
pub(super) async fn run_line(repl: &mut Repl, editor: &mut lineedit::LineEditor, sigint: &mut SigInt, line: &str) -> Flow {
    let mut confirm = EditorConfirm(editor);
    let mut io = Io { interrupt: sigint, confirm: Some(&mut confirm) };
    repl.handle(commands::parse(line), &mut io).await
}

/// Lines from a pipe: one message or command per line, no prompt, no editing. Ctrl-C while
/// waiting for the next line ends the session.
pub(super) async fn pipe_loop(repl: &mut Repl, sigint: &mut SigInt) -> i32 {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        let next = tokio::select! {
            line = lines.next_line() => line,
            _ = sigint.wait() => return 130,
        };
        let line = match next {
            Ok(Some(line)) => line,
            Ok(None) => return 0,
            Err(e) => {
                repl.console.err(&format!("jev-code: cannot read input: {e}"));
                return 1;
            }
        };
        settle_late_probes(repl);
        let mut io = Io { interrupt: sigint, confirm: None };
        match repl.handle(commands::parse(&line), &mut io).await {
            Flow::Continue => {}
            Flow::Quit => return 0,
            Flow::Aborted => return 130,
        }
    }
}
