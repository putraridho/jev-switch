//! The "Ctrl-C twice to exit" rule.

use std::time::{Duration, Instant};

/// A second Ctrl-C within this long of the first exits.
pub const INTERRUPT_WINDOW: Duration = Duration::from_secs(2);

/// What the first Ctrl-C at the prompt says.
pub const EXIT_HINT: &str = "(press Ctrl-C again or Ctrl-D to exit)";

/// What a Ctrl-C at the prompt should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    /// The first one (or one after a pause): clear the line and say how to exit.
    Warn,
    /// A second one within [`INTERRUPT_WINDOW`]: exit.
    Exit,
}

/// Remembers when the last Ctrl-C at the prompt was. Time is passed in, so the rule is a
/// plain unit test.
#[derive(Debug, Default)]
pub struct InterruptGuard {
    last: Option<Instant>,
}

impl InterruptGuard {
    pub fn on_interrupt(&mut self, now: Instant) -> Interrupt {
        let exit = self.last.is_some_and(|t| now.saturating_duration_since(t) <= INTERRUPT_WINDOW);
        self.last = Some(now);
        if exit {
            Interrupt::Exit
        } else {
            Interrupt::Warn
        }
    }

    /// Anything the user typed and sent since: the next Ctrl-C is a first one again.
    pub fn reset(&mut self) {
        self.last = None;
    }
}
