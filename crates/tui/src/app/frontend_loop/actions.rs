//! Carrying out what the screen asked for: send, steer, stop, print, suspend, quit.

use crossterm::event::EventStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::tty_io::{keys_off, keys_on, show};
use super::turn::{start, Done, Stops};
use crate::app::screen_state::{Action, App};
use crate::render::Style;
use crate::repl::session::Repl;
use crate::tty::frontend_screen::Screen;

/// What carrying out actions asks of the loop.
#[derive(Default)]
pub(super) struct Next {
    pub(super) start: Option<JoinHandle<Done>>,
    pub(super) exit: Option<i32>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn carry_out(
    actions: Vec<Action>,
    app: &mut App,
    screen: &mut Screen<std::io::Stdout>,
    events: &mut Option<EventStream>,
    stop_tx: &mpsc::UnboundedSender<()>,
    idle: &mut Option<(Repl, Stops)>,
    mode_picked_during_turn: &mut bool,
    style: Style,
    console: &crate::console::terminal_ui::Console,
) -> Next {
    let mut next = Next::default();
    for action in actions {
        match action {
            // The model may take it into the turn that is running; if it cannot, the message
            // waits in the queue for the next one.
            Action::Steer(text) => {
                console.steer(&text);
            }
            Action::Unsteer(text) => console.withdraw_steer(&text),
            Action::Submit(line) => {
                if let Some(started) = start(idle, &line, style, screen, app) {
                    next.start = Some(started);
                } else {
                    // A line sent while the session is busy waits for it.
                    app.queue.push_back(line);
                }
            }
            Action::Stop => {
                let _ = stop_tx.send(());
            }
            Action::Quit => {
                next.exit = Some(0);
                return next;
            }
            Action::Print(text) => show(screen, app, &text, style),
            Action::Mode(_) => {
                if idle.is_none() {
                    *mode_picked_during_turn = true;
                }
            }
            Action::ClearScreen => {
                if app.fullscreen {
                    app.transcript.clear();
                } else {
                    screen.raw(crate::console::terminal_ui::CLEAR_SCREEN);
                }
                screen.forget();
            }
            Action::Bell => screen.raw("\x07"),
            Action::Suspend(ack) => {
                screen.erase();
                // The key reader must let go of stdin before the command reads it.
                *events = None;
                keys_off();
                let _ = ack.send(());
            }
            Action::Resume => {
                if let Err(e) = keys_on() {
                    eprintln!("jev-code: cannot take the terminal back: {e}");
                    next.exit = Some(1);
                    return next;
                }
                screen.forget();
                *events = Some(EventStream::new());
            }
        }
    }
    next
}
