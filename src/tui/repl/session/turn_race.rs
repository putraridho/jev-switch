//! Racing a turn against Ctrl-C: the first press stops the turn (the engine kills the model
//! CLI and everything it started, and reports `Cancelled`), a second one while it is
//! stopping gives up on waiting.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use crate::tui::console::terminal_ui::Console;
use crate::tui::input::typeahead::TypeAhead;
use crate::turn::application::cancel::Cancel;
use crate::turn::application::engine::Engine;
use crate::turn::application::turn_report::TurnReport;
use crate::turn::domain::turn_status::TurnStatus;

/// A source of Ctrl-C presses.
pub trait Interrupt: Send {
    /// Resolves at the next press.
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// Forget presses that arrived while no turn was running, so a stale one does not stop
    /// the next turn the moment it starts.
    fn drain(&mut self);
}

/// SIGINT as a stream, installed once for the whole run. Installing it early means a
/// Ctrl-C between two turns is queued instead of killing the process (the default action).
#[cfg(unix)]
pub struct SigInt(tokio::signal::unix::Signal);

#[cfg(unix)]
impl SigInt {
    pub fn install() -> std::io::Result<Self> {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map(Self)
    }
}

#[cfg(unix)]
impl Interrupt for SigInt {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // A closed stream never fires: it must not look like a Ctrl-C.
            if self.0.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        })
    }

    fn drain(&mut self) {
        use futures_util::FutureExt;
        while self.0.recv().now_or_never().flatten().is_some() {}
    }
}

#[cfg(not(unix))]
pub struct SigInt;

#[cfg(not(unix))]
impl SigInt {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(not(unix))]
impl Interrupt for SigInt {
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            let _ = tokio::signal::ctrl_c().await;
        })
    }

    fn drain(&mut self) {}
}


/// How a turn ended, as far as the loop cares.
#[derive(Debug)]
pub enum TurnEnd {
    Done(Box<TurnReport>, Duration),
    /// A second Ctrl-C while the turn was stopping: the engine's future was dropped (which
    /// kills the model CLI and its children) and the program should exit.
    Aborted,
}

/// The next Ctrl-C that is meant for the turn. One pressed while an approval's pager is open
/// went to the pager too (it ends a search there) and does not stop the turn.
pub(super) async fn press_outside_pager(interrupt: &mut dyn Interrupt, console: &Console) {
    loop {
        interrupt.wait().await;
        if !console.is_paging() {
            return;
        }
    }
}

/// Which key asked a turn to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopKey {
    CtrlC,
    Esc,
}

/// The next press that stops the turn: Ctrl-C (outside a pager), or Esc when keys are read
/// during the turn.
async fn stop_pressed(interrupt: &mut dyn Interrupt, console: &Console, ahead: Option<&TypeAhead>) -> StopKey {
    let esc = async {
        match ahead {
            Some(a) => a.escaped().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = press_outside_pager(interrupt, console) => StopKey::CtrlC,
        _ = esc => StopKey::Esc,
    }
}

/// Run one turn that Ctrl-C (or Esc) can stop. The status line shows while it runs and is
/// gone when this returns. With type-ahead on, keys typed meanwhile queue messages on the
/// console; a Ctrl-C stop drops that queue (Esc keeps it: it is how a message is sent now).
pub async fn run_turn(engine: &mut Engine, console: &Console, prompt: &str, interrupt: &mut dyn Interrupt) -> TurnEnd {
    interrupt.drain();
    let cancel = Cancel::new();
    let started = Instant::now();
    console.begin_turn();
    let _ticker = console.ticker();
    let ahead = console.type_ahead().then(|| TypeAhead::start(console.clone()));
    let mut ui = console.clone();
    let mut stopped_by: Option<StopKey> = None;
    let report = {
        let turn = engine.run_cancellable(prompt, &mut ui, &cancel);
        tokio::pin!(turn);
        tokio::select! {
            report = &mut turn => Some(report),
            key = stop_pressed(interrupt, console, ahead.as_ref()) => {
                stopped_by = Some(key);
                cancel.cancel();
                console.set_activity("stopping...");
                tokio::select! {
                    report = &mut turn => Some(report),
                    // Dropping `turn` at the end of this block kills what is still running.
                    _ = press_outside_pager(interrupt, console) => None,
                }
            }
        }
    };
    // A pager still open (its question was withdrawn) keeps the terminal until it is closed.
    console.pager_closed().await;
    if let Some(a) = ahead {
        a.finish();
    }
    // Ctrl-C stops a turn as a signal, or as a key at an approval question (which reads keys
    // raw): either way it is not Esc, and the queue goes.
    let cancelled = report.as_ref().is_some_and(|r| r.status == TurnStatus::Cancelled);
    if stopped_by == Some(StopKey::CtrlC) || (cancelled && stopped_by != Some(StopKey::Esc)) {
        let dropped = console.clear_queue();
        if dropped > 0 {
            let noun = if dropped == 1 { "message" } else { "messages" };
            console.err(&format!("  {}", console.err_style().dim(&format!("dropped {dropped} queued {noun} (Ctrl-C); Esc stops and keeps them"))));
        }
    }
    console.end_turn();
    match report {
        Some(r) => TurnEnd::Done(Box::new(r), started.elapsed()),
        None => TurnEnd::Aborted,
    }
}
