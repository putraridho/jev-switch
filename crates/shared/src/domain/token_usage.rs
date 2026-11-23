//! Token usage for a turn; this module performs no IO.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Measurement {
    #[default]
    Reported,
    Unavailable,
}

/// Token counts for one turn. `input_tokens` is everything sent (fresh plus cached,
/// so providers compare on one scale) and `cached_input_tokens` is the cached share.
/// `context_tokens` is the size of the last request, when the CLI reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub measurement: Measurement,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub context_tokens: Option<u64>,
}
