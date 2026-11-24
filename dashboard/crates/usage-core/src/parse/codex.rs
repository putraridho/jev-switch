//! The `account/rateLimits/read` response of `codex app-server` (DESIGN 6.3). T01 writes
//! the bodies.

use crate::label::Label;
use crate::meter::Meters;
use crate::usage::Unavailable;

/// What one rate-limit response says.
#[derive(Clone, PartialEq, Debug)]
pub struct CodexLimits {
    /// The windows and, when present, the credit balance.
    pub meters: Meters,
    /// The plan name from `planType`.
    pub plan: Option<Label>,
}

/// `None` when `line` is not the JSON-RPC response with this id (notifications, other ids).
/// `Some(Err(Protocol))` when it is that response but an error or malformed.
pub fn parse_rate_limits_response(
    line: &[u8],
    id: u64,
) -> Option<Result<CodexLimits, Unavailable>> {
    let _ = (line, id);
    None
}

/// Whether `line` is the JSON-RPC response with this id (for the `initialize` ack).
pub fn is_response(line: &[u8], id: u64) -> bool {
    let _ = (line, id);
    false
}
