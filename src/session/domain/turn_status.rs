//! How a turn ended.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// A model answered; see [`TurnReport::text`].
    Answered,
    /// No usable model could take the turn, so nothing was started.
    NoRoute,
    /// A model was started and failed in a way that ended the turn.
    Failed,
    /// The user stopped the turn. Nothing was retried, routing and cooldowns are
    /// unchanged, and the next turn is told this one was interrupted.
    Cancelled,
}
