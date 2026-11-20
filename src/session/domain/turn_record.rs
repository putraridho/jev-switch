//! One turn as the hand-off remembers it, and the notes that stand in for turns without an answer.

use crate::shared::ProviderId;

/// One finished (or failed) turn, as the hand-off needs to remember it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub prompt: String,
    pub provider: ProviderId,
    pub model: String,
    pub reply: String,
    /// `Event::Tool` summaries seen during the turn, in order.
    pub tools: Vec<String>,
    pub ok: bool,
}

/// What the next catch-up says about an undone turn.
pub const UNDO_NOTE: &str = "(the user undid the changes of the previous turn)";

/// What a turn that the user cancelled says in the next catch-up.
pub const INTERRUPTED_NOTE: &str = "(interrupted by the user before it finished; files may be partly changed)";
