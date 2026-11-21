use super::*;
use crate::backend::classify_common;
use crate::backend::infrastructure::grok::grok_cli::errors::{is_auth_text, is_entitlement_text};

#[test]
fn auth_failure_is_failed_and_classified_auth() {
    let events = parse_all(ERR_AUTH);
    let [Event::Failed { message }] = &events[..] else { panic!("{events:?}") };
    assert!(message.contains("grok login"));
    assert_eq!(backend().classify_error(message), ErrorKind::Auth);
}

#[test]
fn plan_refusal_is_not_entitled() {
    let events = parse_all(ERR_PLAN);
    let [Event::Failed { message }] = &events[..] else { panic!("{events:?}") };
    assert!(message.contains("SuperGrok"));
    assert_eq!(backend().classify_error(message), ErrorKind::NotEntitled);
    assert_eq!(backend().classify_error("A SuperGrok subscription is required"), ErrorKind::NotEntitled);
}


#[test]
fn classifies_common_errors_and_unknowns() {
    assert!(matches!(backend().classify_error("HTTP 429 Too Many Requests"), ErrorKind::Quota { .. }));
    assert_eq!(backend().classify_error("weird"), ErrorKind::Other);
}


#[test]
fn billing_words_alone_do_not_mean_the_plan_gate_or_a_login_problem() {
    assert!(!is_entitlement_text("update the subscription handler to apply premium pricing"));
    assert!(!is_auth_text("the api key field is rendered in the settings form"));
}

#[test]
fn entitlement_needs_a_plan_phrase_not_common_word_pairs() {
    for text in [
        "error: subscription id is required",
        "premium pricing handler",
        "the node version is old and a subscription is required by the billing api",
        "the file is not included in your tsconfig",
        "your plan is to upgrade the subscription module",
    ] {
        assert!(!is_entitlement_text(&text.to_lowercase()), "{text:?}");
        assert_eq!(backend().classify_error(text), ErrorKind::Other, "{text:?}");
    }
    for text in [
        "Grok Build is not included in your X Premium plan. A SuperGrok or X Premium+ subscription is required.",
        "Grok requires a SuperGrok plan",
        "Upgrade to SuperGrok to use grok build",
        "A SuperGrok subscription is required",
        "grok: subscription required",
        "No active subscription for Grok",
    ] {
        assert!(is_entitlement_text(&text.to_lowercase()), "{text:?}");
        assert_eq!(backend().classify_error(text), ErrorKind::NotEntitled, "{text:?}");
    }
}

#[test]
fn auth_needs_a_login_phrase_not_common_word_pairs() {
    for text in [
        "api key validation error",
        "the xai_api_key variable is documented in the readme",
        "authentication module has an error",
        "api key rotation failed in the billing job",
    ] {
        assert!(!is_auth_text(&text.to_lowercase()), "{text:?}");
        assert_eq!(backend().classify_error(text), ErrorKind::Other, "{text:?}");
    }
    for text in [
        "Couldn't start session: not authenticated. Run `grok login` or set XAI_API_KEY.",
        "api key missing: run grok login",
        "XAI_API_KEY is not set",
        "XAI_API_KEY invalid",
        "the Grok API key is missing",
    ] {
        assert!(is_auth_text(&text.to_lowercase()) || classify_common(text).is_some(), "{text:?}");
        assert_eq!(backend().classify_error(text), ErrorKind::Auth, "{text:?}");
    }
}
