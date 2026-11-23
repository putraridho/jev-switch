use super::*;
use ProviderId::{Claude, Codex, DeepSeek, Grok};

#[test]
fn access_decides_custom_then_key_then_login() {
    let all = Access { custom_endpoint: true, api_key: true, login: true };
    assert_eq!(Funding::from_access(all), Funding::Custom);
    assert_eq!(Funding::from_access(Access { api_key: true, login: true, ..Access::default() }), Funding::ApiKey);
    assert_eq!(Funding::from_access(Access { login: true, ..Access::default() }), Funding::Login);
    assert_eq!(Funding::from_access(Access::default()), Funding::Unknown);
}

#[test]
fn labels_do_not_claim_a_subscription() {
    assert_eq!(Funding::Login.label(Codex), "ChatGPT login");
    assert_eq!(Funding::Custom.label(Codex), "custom endpoint (billing unknown)");
    assert_eq!(Funding::ApiKey.label(Grok), "API key (per token)");
    assert_eq!(Funding::Unknown.label(Claude), "billing unknown");
}

#[test]
fn a_native_login_keeps_the_catalog_billing() {
    let f = Fundings::default();
    assert!(!f.is_paid(Codex, "gpt-6.1-terra"), "not inspected: as the catalog says");
    assert!(!Fundings::default().with(Codex, Funding::Login).is_paid(Codex, "gpt-6.1-terra"));
}

#[test]
fn custom_key_and_unknown_funding_are_paid_controlled() {
    for funding in [Funding::Custom, Funding::ApiKey, Funding::Unknown] {
        let f = Fundings::default().with(Codex, funding);
        assert!(f.is_paid(Codex, "gpt-6.1-terra"), "{funding:?}");
        assert!(!f.is_paid(Claude, "claude-sonnet-5-5"), "other providers are unaffected");
    }
}

#[test]
fn per_token_and_unknown_models_stay_paid() {
    assert!(Fundings::default().with(DeepSeek, Funding::ApiKey).is_paid(DeepSeek, "deepseek-flash"));
    assert!(Fundings::default().is_paid(Codex, "not-a-model"));
}
