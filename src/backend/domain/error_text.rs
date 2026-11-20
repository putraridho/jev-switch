//! Reading a CLI's error text: which failures callers act on, and the reset time of a limit.

use super::error_kind::ErrorKind;
use crate::shared::domain::limit_notice::is_limit_phrase;

/// True when `needle` (a number) appears in `hay` not glued to other digits or
/// letters, so "429" matches "HTTP 429" but not "a4291b".
pub(crate) fn has_token(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The "resets ..." tail of a limit message ("resets 3pm", "resets Nov 24", "resets 5pm
/// (Asia/Jakarta)"), or the epoch in the older "... limit reached|<epoch>" form. Kept
/// as the CLI's own wording.
pub(crate) fn resets_tail(message: &str) -> Option<String> {
    // ASCII lowercasing keeps byte offsets identical to the original.
    let lower = message.to_ascii_lowercase();
    if let Some(i) = lower.find("resets") {
        let tail = message[i + "resets".len()..].lines().next().unwrap_or("");
        let tail = tail.trim().trim_start_matches([':', '-']).trim();
        return (!tail.is_empty()).then(|| tail.chars().take(80).collect());
    }
    let (_, epoch) = lower.split_once("limit reached|")?;
    let digits: String = epoch.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty()).then_some(digits)
}

/// True when `lower` contains any of `phrases`.
pub(crate) fn has_any(lower: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| lower.contains(p))
}

/// Matches only PHRASES a CLI uses for these failures, never a bare word: error
/// text routinely mentions "quota", "upgrade" or "expired" in other senses, and a
/// wrong match would mark a provider unusable or stop failover.
fn is_quota_text(lower: &str) -> bool {
    is_limit_phrase(lower)
        || has_any(lower, &["resource_exhausted", "too many requests"])
        || (lower.contains("rate limit")
            && (has_any(lower, &["exceed", "reached", "too many"]) || has_token(lower, "hit")))
        || (lower.contains("quota") && has_any(lower, &["exceed", "exhaust", "reached", "insufficient"]))
        || has_token(lower, "429")
}

fn is_entitlement_text(lower: &str) -> bool {
    let about_the_plan = has_any(lower, &["plan", "subscription", "tier", "account"]);
    has_any(lower, &["not supported when using", "not available on your plan"])
        || (about_the_plan && has_any(lower, &["upgrade your", "upgrade to a", "not included in your"]))
}

fn is_auth_text(lower: &str) -> bool {
    has_token(lower, "401")
        || has_token(lower, "unauthorized")
        || has_any(lower, &["not logged in", "please log in", "invalid api key", "login expired", "session expired"])
        || has_any(lower, &["authentication failed", "authentication required", "authentication error"])
        || (lower.contains("token") && lower.contains("expired"))
}

/// "I will not run here": a workspace-trust or git-repository check. Codex says
/// "Not inside a trusted directory and --skip-git-repo-check was not specified" and
/// Gemini CLI "is not running in a trusted directory ... --skip-trust".
fn is_unusable_here_text(lower: &str) -> bool {
    ["trusted directory", "trusted folder", "trusted workspace", "untrusted", "skip-git-repo-check", "skip-trust"]
        .iter()
        .any(|p| lower.contains(p))
}

/// Phrases every CLI uses for the errors callers act on. Backends call this first
/// so quota, plan, login and "cannot run here" failures read the same whichever
/// CLI said them.
pub fn classify_common(message: &str) -> Option<ErrorKind> {
    let lower = message.to_lowercase().replace('\u{2019}', "'");
    if is_quota_text(&lower) {
        Some(ErrorKind::Quota { resets_at: resets_tail(message) })
    } else if is_entitlement_text(&lower) {
        Some(ErrorKind::NotEntitled)
    } else if is_auth_text(&lower) {
        Some(ErrorKind::Auth)
    } else if is_unusable_here_text(&lower) {
        Some(ErrorKind::UnusableHere)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_quota_phrases() {
        let quota = |m: &str| matches!(classify_common(m), Some(ErrorKind::Quota { .. }));
        assert!(quota("You've hit your weekly limit"));
        assert!(quota("Usage limit reached"));
        assert!(quota("rate limit exceeded"));
        assert!(quota("Quota exceeded for this project"));
        assert!(quota("HTTP 429 Too Many Requests"));
        assert_eq!(
            classify_common("You've hit your session limit · resets 5pm (Asia/Jakarta)"),
            Some(ErrorKind::Quota { resets_at: Some("5pm (Asia/Jakarta)".into()) })
        );
        assert_eq!(
            classify_common("Claude AI usage limit reached|1794853451"),
            Some(ErrorKind::Quota { resets_at: Some("1794853451".into()) })
        );
        assert_eq!(classify_common("quota exceeded"), Some(ErrorKind::Quota { resets_at: None }));
    }

    #[test]
    fn real_claude_limit_notices_are_quota_with_their_reset_time() {
        let quota = |resets: Option<&str>| Some(ErrorKind::Quota { resets_at: resets.map(String::from) });
        for (text, resets) in [
            ("5-hour limit reached \u{2219} resets 3pm", Some("3pm")),
            ("Weekly limit reached \u{2219} resets Nov 24", Some("Nov 24")),
            ("Opus weekly limit reached", None),
            ("You've hit your session limit \u{b7} resets 5pm (Asia/Jakarta)", Some("5pm (Asia/Jakarta)")),
            ("You\u{2019}ve hit your session limit \u{b7} resets 5pm (Asia/Jakarta)", Some("5pm (Asia/Jakarta)")),
            ("Claude AI usage limit reached|1794853451", Some("1794853451")),
            ("5-hour limit reached|1794853451", Some("1794853451")),
            ("Approaching usage limit", None),
            ("limit reached", None),
            ("You have reached your weekly limit", None),
            ("You've reached your usage limit \u{b7} resets 9am", Some("9am")),
            ("monthly limit \u{2219} resets Dec 1", Some("Dec 1")),
        ] {
            assert_eq!(classify_common(text), quota(resets), "{text:?}");
        }
    }

    #[test]
    fn common_entitlement_and_auth_phrases() {
        assert_eq!(classify_common("model is not supported when using ChatGPT"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("Not available on your plan"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("Please upgrade your plan to continue"), Some(ErrorKind::NotEntitled));
        assert_eq!(classify_common("401 Unauthorized"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("Not logged in"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("token expired"), Some(ErrorKind::Auth));
        assert_eq!(classify_common("Please log in again"), Some(ErrorKind::Auth));
    }

    #[test]
    fn cannot_run_here_wording_is_unusable_here() {
        let here = |m: &str| classify_common(m) == Some(ErrorKind::UnusableHere);
        // Codex outside a git repository.
        assert!(here("Not inside a trusted directory and --skip-git-repo-check was not specified."));
        // Gemini CLI in an untrusted workspace.
        assert!(here(
            "Gemini CLI is not running in a trusted directory. To proceed, either use `--skip-trust`, set the \
             `GEMINI_CLI_TRUST_WORKSPACE=true` environment variable, or trust this directory in interactive mode."
        ));
        assert!(here("this is an untrusted workspace"));
        // Quota and login wording still win over it.
        assert!(matches!(classify_common("usage limit, resets 5pm"), Some(ErrorKind::Quota { .. })));
        assert_eq!(classify_common("please log in"), Some(ErrorKind::Auth));
    }

    #[test]
    fn common_ignores_unrelated_text() {
        assert_eq!(classify_common("segmentation fault"), None);
        assert_eq!(classify_common("read 14291 bytes"), None);
        assert_eq!(classify_common(""), None);
    }

    #[test]
    fn common_matches_phrases_and_never_bare_words() {
        use ErrorKind::{Auth, NotEntitled};
        let kind = |m: &str| classify_common(m).map(|k| match k {
            ErrorKind::Quota { .. } => "quota",
            NotEntitled => "entitled",
            Auth => "auth",
            ErrorKind::UnusableHere => "here",
            _ => "other",
        });
        let should_match = [
            ("quota", "You have hit your usage limit"),
            ("quota", "You've hit your session limit"),
            ("quota", "rate limit exceeded"),
            ("quota", "Rate limit reached for requests"),
            ("quota", "we hit the rate limit"),
            ("quota", "quota exceeded"),
            ("quota", "you have exhausted your quota"),
            ("quota", "insufficient quota"),
            ("quota", "RESOURCE_EXHAUSTED: try later"),
            ("quota", "Too Many Requests"),
            ("quota", "status 429"),
            ("entitled", "model is not supported when using ChatGPT"),
            ("entitled", "Not available on your plan"),
            ("entitled", "Upgrade your subscription to use this model"),
            ("entitled", "upgrade to a paid tier"),
            ("entitled", "Grok Build is not included in your X Premium plan"),
            ("auth", "HTTP 401"),
            ("auth", "Unauthorized"),
            ("auth", "Not logged in"),
            ("auth", "Please log in again"),
            ("auth", "Invalid API key"),
            ("auth", "authentication failed or timed out"),
            ("auth", "Authentication required"),
            ("auth", "the OAuth token has expired"),
            ("auth", "login expired"),
            ("auth", "Your session expired"),
        ];
        for (want, text) in should_match {
            assert_eq!(kind(text), Some(want), "{text:?}");
        }
        let must_not_match = [
            "the upgrade guide is in docs/upgrading.md",
            "upgrade the dependency to 2.0",
            "expired cache entries were removed",
            "quota calculation function",
            "authenticate the user in the handler",
            "authentication module not found",
            "the rate limit middleware is missing",
            "rate limit handling for the client",
            "when the timer resets the counter goes to zero",
            "resets the state of the form",
            "the resets handler",
            "limit the query to 10 rows",
            "reached your destination in 10 minutes",
            "file not included in the build",
            "error 4010 in module",
            "request id 14290",
        ];
        for text in must_not_match {
            assert_eq!(kind(text), None, "{text:?}");
        }
    }
}
