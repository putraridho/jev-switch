//! The model chosen for one attempt, and one started attempt with what came of it.

use jev_backend::{McpAttachment, TurnError, TurnOutcome};
use jev_catalog::domain::model_table::ModelSpec;
use jev_routing::domain::decide::RoutingState;
use crate::domain::route_facts::RouteFacts;

/// The model chosen for one attempt.
pub(super) struct Chosen {
    pub(super) spec: &'static ModelSpec,
    pub(super) reason: String,
    /// The same decision with its parts, for [`Ui::route_facts`].
    pub(super) facts: RouteFacts,
    /// The routing state after the decision; kept only if the attempt succeeds.
    pub(super) state: RoutingState,
    /// Legacy mode only: the tier index, to feed back into [`Sticky`].
    pub(super) legacy_tier: Option<usize>,
    /// Orchestration: the delegate tool to attach, when this model is the planner.
    pub(super) mcp: Option<McpAttachment>,
}

/// The prompt with what the person added while the turn ran, as the turn (and a retry of it)
/// has it.
pub(super) fn with_steered(prompt: &str, steered: &[String]) -> String {
    steered.iter().fold(prompt.to_string(), |all, text| format!("{all}\n\n(added while it ran) {text}"))
}

/// One started attempt and what came of it.
pub(super) struct Attempt {
    pub(super) chosen: Chosen,
    pub(super) tools: Vec<String>,
    /// Messages the person typed during the attempt that the model took into the turn.
    pub(super) steered: Vec<String>,
    pub(super) wall_ms: u64,
    pub(super) result: Result<TurnOutcome, TurnError>,
    /// The attempt failed after it had produced text, which was therefore never shown.
    pub(super) discarded: bool,
    /// The user cancelled it; `result` then holds a placeholder error that is never acted on.
    pub(super) cancelled: bool,
    /// Tool outputs that said they left processes running.
    pub(super) background_results: usize,
    /// The backend already said that work was left running.
    pub(super) background_noticed: bool,
}
