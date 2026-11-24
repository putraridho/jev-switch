//! One line of `claude -p --output-format stream-json` (DESIGN 6.2). T01 writes the body.

use crate::meter::Meters;

/// `Some` only for a `rate_limit_event` line.
pub fn parse_line(line: &[u8]) -> Option<Meters> {
    let _ = line;
    None
}
