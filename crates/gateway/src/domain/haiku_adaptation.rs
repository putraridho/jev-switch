//! When Haiku's answer means "retry on Sonnet", and which betas it cannot take.

/// A Haiku response that means "not this request, not now": the settings or size it
/// rejects (400, 413, 422) or its own rate limit (429, seen on title and summary requests
/// while the account still had quota on the other models). The request is retried on Sonnet.
pub fn haiku_declined(status: u16) -> bool {
    matches!(status, 400 | 413 | 422 | 429)
}

pub fn without_long_context(beta: &str) -> String {
    beta.split(',').map(str::trim).filter(|b| !b.is_empty() && !b.starts_with("context-1m")).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haiku_rejections_and_rate_limits_fall_back_to_sonnet() {
        for s in [400, 413, 422, 429] {
            assert!(haiku_declined(s), "{s}");
        }
        for s in [200, 401, 404, 500, 529] {
            assert!(!haiku_declined(s), "{s}");
        }
    }

    #[test]
    fn long_context_beta_is_stripped_from_the_list() {
        assert_eq!(
            without_long_context("claude-code-20250219,oauth-2025-04-20,context-1m-2025-08-07,interleaved-thinking-2025-05-14"),
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14"
        );
        assert_eq!(without_long_context("context-1m-2025-08-07"), "");
    }
}
