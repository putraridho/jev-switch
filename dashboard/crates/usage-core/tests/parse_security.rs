//! Parser fixture and security checks.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
use jev_usage_core::parse::{claude_auth, claude_event, claude_log, codex, deepseek};
use proptest::prelude::*;

fn check(bytes: &[u8]) {
    let output = format!(
        "{:?}{:?}{:?}{:?}{:?}",
        claude_auth::parse(bytes),
        claude_event::parse_line(bytes),
        claude_log::parse_line(bytes),
        codex::parse_rate_limits_response(bytes, 2),
        deepseek::parse_balance(bytes)
    );
    assert!(!output.contains("JEVCANARY"));
    assert!(!output.contains("canary-email"));
}
#[test]
fn fixture_canaries_do_not_surface() {
    for bytes in [
        include_bytes!("../../../fixtures/claude-gateway/canary.jsonl").as_slice(),
        include_bytes!("../../../fixtures/claude-auth/logged_in.json").as_slice(),
        include_bytes!("../../../fixtures/codex-app-server/account_read.response.json").as_slice(),
        include_bytes!("../../../fixtures/deepseek/balance_canary.json").as_slice(),
    ] {
        for line in bytes.split(|b| *b == b'\n') {
            check(line);
        }
    }
}
#[test]
fn fixtures_produce_meters_and_login() {
    let row = include_bytes!("../../../fixtures/claude-gateway/sample.jsonl")
        .split(|b| *b == b'\n')
        .next()
        .unwrap();
    assert_eq!(
        claude_log::parse_line(row)
            .unwrap()
            .windows
            .as_slice()
            .len(),
        2
    );
    assert_eq!(
        codex::parse_rate_limits_response(
            include_bytes!("../../../fixtures/codex-app-server/paid_plan.response.json"),
            2
        )
        .unwrap()
        .unwrap()
        .meters
        .as_slice()
        .len(),
        3
    );
    assert_eq!(
        deepseek::parse_balance(include_bytes!(
            "../../../fixtures/deepseek/balance_two_currencies.json"
        ))
        .unwrap()
        .as_slice()
        .len(),
        2
    );
    assert_eq!(
        claude_auth::parse(include_bytes!(
            "../../../fixtures/claude-auth/logged_in.json"
        ))
        .unwrap()
        .plan
        .unwrap()
        .as_str(),
        "max_5x"
    );
}
proptest! {
    #[test]
    fn arbitrary_bytes_do_not_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        check(&bytes);
        if claude_log::parse_line(&bytes).is_some() { prop_assert!(claude_log::might_have_sample(&bytes)); }
    }
}
