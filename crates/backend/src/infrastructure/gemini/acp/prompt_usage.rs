//! The consumption an ACP agent reports with a finished prompt (`result.usage`, the protocol's
//! optional `inputTokens` / `outputTokens` / `cachedReadTokens`). Agents that send none keep
//! the turn's consumption unavailable, which is not the same as zero.

use serde_json::Value;

use crate::{Measurement, Usage};

/// The counts in `usage`, or `None` unless it names both input and output tokens as numbers
/// (a half-reported turn would understate what it consumed). `inputTokens` is taken as the
/// whole prompt including the cached share, so the cached count is capped to it.
pub(super) fn reported(usage: &Value, context_tokens: Option<u64>,
    window_tokens: Option<u32>) -> Option<Usage> {
    let input_tokens = usage["inputTokens"].as_u64()?;
    let output_tokens = usage["outputTokens"].as_u64()?;
    let cached_input_tokens = usage["cachedReadTokens"].as_u64().unwrap_or(0).min(input_tokens);
    Some(Usage { measurement: Measurement::Reported, input_tokens, cached_input_tokens, output_tokens, context_tokens,
        cache_written_tokens: None,
        window_tokens })
}
