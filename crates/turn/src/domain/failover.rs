//! Why an attempt failed over, or why nothing could be started, as the UI is told.

use jev_backend::{ErrorKind, ProviderId};

/// A failed attempt, as the UI is told about it.
#[derive(Debug, Clone, Copy)]
pub struct Failover<'a> {
    pub provider: ProviderId,
    pub model: &'a str,
    pub kind: &'a ErrorKind,
    /// What the CLI said.
    pub message: &'a str,
    /// The failed attempt had already produced text, which was dropped.
    pub discarded: bool,
    /// True: another model is tried next. False: the turn ends here with this error.
    pub will_retry: bool,
    /// The engine's own one-line wording, for UIs that only print text.
    pub text: &'a str,
}

/// Why nothing could be started, as the UI is told about it.
#[derive(Debug, Clone, Copy)]
pub enum NoRouteInfo<'a> {
    /// A `/model` pin names a model that cannot run now.
    Pin { what: &'a str },
    /// No usable model fits this kind of task.
    Nothing { task: &'a str },
    /// Your privacy policy, budget cap or routing rules left no model.
    Refused { why: &'a str },
}
