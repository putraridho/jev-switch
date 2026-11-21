//! Running a line on a task of its own: its stop channel, its confirmations, its start.

use std::future::Future;
use std::pin::Pin;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::tty_io::{show, text_width};
use crate::app::screen_state::App;
use crate::command as commands;
use crate::console::front::Front;
use crate::render::Style;
use crate::repl::session::turn_race::Interrupt;
use crate::repl::session::{Confirm, Flow, Io, Repl};
use crate::tty::frontend_screen::Screen;

/// Stop requests from the screen, as the session's [`Interrupt`].
pub(super) struct Stops(pub(super) mpsc::UnboundedReceiver<()>);

impl Interrupt for Stops {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // A closed channel never fires: it must not look like a stop.
            if self.0.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        })
    }

    fn drain(&mut self) {
        while self.0.try_recv().is_ok() {}
    }
}

/// Yes/no questions from commands, answered on a card.
struct CardConfirm(Option<Front>);

impl Confirm for CardConfirm {
    fn confirm(&mut self, question: &str) -> bool {
        let question = question.trim_end().trim_end_matches("[y/N]").trim_end();
        self.0.as_ref().is_some_and(|f| f.confirm(question))
    }
}

/// A finished line: the session back, its stop channel back, and what to do next.
pub(super) type Done = (Repl, Stops, Flow);

/// Start running `line` on its own task, if the session is idle. The line is printed above
/// first, as it stays in the scrollback.
pub(super) fn start(idle: &mut Option<(Repl, Stops)>, line: &str, style: Style, screen: &mut Screen<std::io::Stdout>, app: &mut App) -> Option<JoinHandle<Done>> {
    let (mut repl, mut stops) = idle.take()?;
    // Any line that runs answers the last answer, however it was sent (Enter, the queue,
    // Esc during a turn): its offer no longer stands, even when the line brings no answer.
    app.suggestion = None;
    let first = app.sent == 0;
    app.sent += 1;
    let stamp = crate::render::local_clock();
    let block = crate::render::sent_block(line, first, stamp.as_deref(), style, text_width(app));
    show(screen, app, &block, style);
    let line = line.to_string();
    Some(tokio::spawn(async move {
        let mut confirm = CardConfirm(repl.console.front());
        let flow = {
            let mut io = Io { interrupt: &mut stops, confirm: Some(&mut confirm) };
            repl.handle(commands::parse(&line), &mut io).await
        };
        (repl, stops, flow)
    }))
}
