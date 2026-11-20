//! What the router decided for one attempt, as plain strings.

/// What the router decided for one attempt, as plain strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteFacts {
    pub provider: String,
    pub model: String,
    /// The task type, used when `reason` does not carry one.
    pub task: String,
    /// The engine's long reason, e.g. `task: explain (0.91) → claude sonnet-5-5; first turn`.
    pub reason: String,
    pub switched_provider: bool,
    pub dropped_tier: bool,
}
