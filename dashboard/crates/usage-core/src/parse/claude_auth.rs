//! Allow-list for Claude login status.
use crate::label::Label;
use serde::Deserialize;

/// The non-identifying login facts.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ClaudeLogin {
    /// Whether the CLI is signed in.
    pub logged_in: bool,
    /// The plan name, when present and valid.
    pub plan: Option<Label>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status<'a> {
    logged_in: bool,
    #[serde(borrow)]
    plan_tier: Option<&'a str>,
    #[serde(borrow)]
    subscription_type: Option<&'a str>,
}

/// Parse the three allowed JSON fields, or the text login status.
pub fn parse(output: &[u8]) -> Option<ClaudeLogin> {
    if let Ok(status) = serde_json::from_slice::<Status<'_>>(output) {
        let plan = status
            .plan_tier
            .or(status.subscription_type)
            .and_then(|value| {
                let normalized = value.to_ascii_lowercase().replace(['-', ' '], "_");
                Label::parse(&normalized)
            });
        return Some(ClaudeLogin {
            logged_in: status.logged_in,
            plan,
        });
    }
    let contains = |needle: &[u8]| {
        output
            .windows(needle.len())
            .any(|part| part.eq_ignore_ascii_case(needle))
    };
    let logged_in = if contains(b"not logged in") || contains(b"not authenticated") {
        false
    } else if contains(b"logged in") {
        true
    } else {
        return None;
    };
    Some(ClaudeLogin {
        logged_in,
        plan: None,
    })
}
