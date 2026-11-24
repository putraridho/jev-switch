//! One row of the gateway's `usage.jsonl` (DESIGN 6.1). T01 writes the bodies.

use crate::meter::Meters;
use crate::time::UnixMillis;

/// The usage windows of one log row.
#[derive(Clone, PartialEq, Debug)]
pub struct ClaudeLogSample {
    /// The row's `ts_ms`: when the request started.
    pub at: UnixMillis,
    /// The row's windows: at least one window meter.
    pub windows: Meters,
}

/// Cheap byte test used before JSON parsing: could this line contain a sample?
pub fn might_have_sample(line: &[u8]) -> bool {
    let _ = line;
    false
}

/// One line of `usage.jsonl`. `None` for anything that is not a row with at least one
/// `anthropic-ratelimit-unified-<window>-utilization` value. Never panics on any input.
pub fn parse_line(line: &[u8]) -> Option<ClaudeLogSample> {
    let _ = line;
    None
}
