//! Which models a turn may go to: the pure rules over what the probes found, the
//! cooldowns in force and the models a provider refused.

use std::collections::HashSet;

use crate::backend::ProviderId;
use crate::catalog::domain::model_table as catalog;
use crate::catalog::domain::model_table::{Allowances, Usable};
use crate::providers::domain::availability;
use crate::providers::domain::availability::{Availability, State};
use crate::providers::domain::reset_time::Cooldowns;

/// Credits are opt-in via `JEV_ALLOW_CREDITS` ("1" or "true"). Per-token models are
/// allowed exactly when DeepSeek is Ready, because configuring its key is the
/// user's agreement to pay per token.
pub fn allowances_for(credits_env: Option<&str>, avail: &[Availability]) -> Allowances {
    Allowances {
        // Extra-credit models (Claude Fable) are free to use when the plan includes them:
        // Claude Max and Ultra do, Pro bills them as pay-as-you-go credits (opt-in).
        credits: credits_env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
            || claude_plan_includes_fable(avail),
        per_token: deepseek_ready(avail),
    }
}

/// Whether the logged-in Claude plan includes Fable (Max and Ultra do; Pro does not).
fn claude_plan_includes_fable(avail: &[Availability]) -> bool {
    avail.iter().any(|a| {
        a.provider == ProviderId::Claude
            && a.state == State::Ready
            && a.plan.as_deref().is_some_and(|p| {
                let p = p.to_ascii_lowercase();
                p.contains("max") || p.contains("ultra")
            })
    })
}

fn deepseek_ready(avail: &[Availability]) -> bool {
    avail.iter().any(|a| a.provider == ProviderId::DeepSeek && a.state == State::Ready)
}

/// The models a turn may go to: those of Ready providers (narrowed by the login's
/// own model list), minus providers cooling down and models a provider refused.
pub fn usable_from(
    avail: &[Availability],
    cooldowns: &Cooldowns,
    unusable: &HashSet<(ProviderId, String)>,
    now_ms: u64,
) -> Usable {
    let catalog_ids = |p: ProviderId| {
        catalog::models().iter().filter(|m| m.provider == p).map(|m| m.id.to_string()).collect::<Vec<_>>()
    };
    Usable::new(
        availability::usable::usable_models(avail, &catalog_ids)
            .into_iter()
            .filter(|(p, _)| cooldowns.active(*p, now_ms).is_none())
            .filter(|pair| !unusable.contains(pair)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::DeepSeek;

    fn availability(provider: ProviderId, state: State) -> Availability {
        Availability { provider, state, plan: None, models: None, detail: String::new(), verified: true }
    }

    #[test]
    fn allowances_follow_credits_env_and_deepseek_key() {
        let ready = [availability(DeepSeek, State::Ready)];
        let no_key = [availability(DeepSeek, State::NoKey)];
        assert_eq!(allowances_for(None, &ready), Allowances { credits: false, per_token: true });
        assert_eq!(allowances_for(Some("1"), &no_key), Allowances { credits: true, per_token: false });
        assert!(allowances_for(Some("true"), &[]).credits);
        assert!(allowances_for(Some(" TRUE "), &[]).credits);
        assert!(!allowances_for(Some("0"), &[]).credits);
        assert!(!allowances_for(Some("yes"), &[]).credits);
        assert!(!allowances_for(Some(""), &[]).credits);
    }

    #[test]
    fn a_max_or_ultra_claude_plan_allows_fable_but_pro_does_not() {
        let claude = |plan: &str| Availability {
            provider: ProviderId::Claude,
            state: State::Ready,
            plan: Some(plan.into()),
            models: None,
            detail: String::new(),
            verified: true,
        };
        assert!(allowances_for(None, &[claude("max_5x")]).credits);
        assert!(allowances_for(None, &[claude("Ultra")]).credits);
        assert!(!allowances_for(None, &[claude("pro")]).credits);
        assert!(allowances_for(Some("1"), &[claude("pro")]).credits, "the env opt-in still works on Pro");
    }
}
