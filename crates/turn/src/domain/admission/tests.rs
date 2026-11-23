use super::*;
use crate::domain::budget::{Allowance, PriceTable};
use crate::domain::usage_ledger::{Held, Reserved};
use jev_shared::Measurement;

fn paid(tokens: u64) -> Attempt<'static> {
    static CAPPED: Budget = Budget { cap: Some(Cap::Tokens(1_000)), daily: None, allowance: None };
    Attempt { provider: ProviderId::DeepSeek, paid: true, estimate: Estimate { tokens, usd: Some(0.5) }, budget: &CAPPED }
}

fn meter_with(tokens: u64) -> Meter {
    let mut meter = Meter::default();
    meter.record(ProviderId::DeepSeek, "deepseek-flash", &Usage { input_tokens: tokens, ..Usage::default() }, &PriceTable::default());
    meter
}

fn reserved(tokens: u64) -> Reserved {
    let mut r = Reserved::default();
    r.add(&Held { id: "h".into(), owner_pid: 1, owner_start: None, ts_ms: 0, provider: ProviderId::DeepSeek, model: "deepseek-flash".into(), paid: true, tokens, usd: None });
    r
}

fn standing<'a>(session: &'a Meter, held: &'a Reserved) -> Standing<'a> {
    Standing { session, session_reserved: held, day: session, day_reserved: held }
}

#[test]
fn a_paid_attempt_is_refused_when_spent_plus_reserved_plus_its_estimate_would_pass_the_cap() {
    let none = Reserved::default();
    assert!(matches!(admit(&paid(300), &standing(&meter_with(600), &none)), Admission::Admitted { .. }), "600 + 300 fits 1000");
    assert!(matches!(admit(&paid(500), &standing(&meter_with(600), &none)), Admission::Refused(_)), "600 + 500 does not");
    let held = reserved(300);
    assert!(matches!(admit(&paid(300), &standing(&Meter::default(), &held)), Admission::Admitted { .. }));
    let Admission::Refused(why) = admit(&paid(800), &standing(&meter_with(100), &held)) else { panic!("refused") };
    assert!(why.contains("session cap") && why.contains("reserved"), "{why}");
}

#[test]
fn a_cap_already_reached_refuses_even_a_tiny_attempt() {
    let none = Reserved::default();
    assert!(matches!(admit(&paid(0), &standing(&meter_with(1_000), &none)), Admission::Refused(_)));
}

#[test]
fn unreported_paid_use_fails_closed() {
    let mut meter = Meter::default();
    meter.record(ProviderId::DeepSeek, "deepseek-flash", &Usage { measurement: Measurement::Unavailable, ..Usage::default() }, &PriceTable::default());
    assert!(matches!(admit(&paid(1), &standing(&meter, &Reserved::default())), Admission::Refused(_)));
}

#[test]
fn a_dollar_cap_needs_a_price_for_the_attempt() {
    let budget = Budget::capped(Cap::Usd(5.0));
    let attempt = Attempt { provider: ProviderId::DeepSeek, paid: true, estimate: Estimate { tokens: 10, usd: None }, budget: &budget };
    let none = Reserved::default();
    let Admission::Refused(why) = admit(&attempt, &standing(&Meter::default(), &none)) else { panic!("refused") };
    assert!(why.contains("prices.txt"), "{why}");
    let priced = Attempt { estimate: Estimate { tokens: 10, usd: Some(1.0) }, ..attempt };
    assert!(matches!(admit(&priced, &standing(&Meter::default(), &none)), Admission::Admitted { .. }));
    let too_much = Attempt { estimate: Estimate { tokens: 10, usd: Some(6.0) }, ..attempt };
    assert!(matches!(admit(&too_much, &standing(&Meter::default(), &none)), Admission::Refused(_)));
}

#[test]
fn the_daily_cap_counts_every_session() {
    let budget = Budget { daily: Some(Cap::Tokens(1_000)), ..Budget::default() };
    let attempt = Attempt { budget: &budget, ..paid(100) };
    let (session, day, none) = (Meter::default(), meter_with(950), Reserved::default());
    let now = Standing { session: &session, session_reserved: &none, day: &day, day_reserved: &none };
    let Admission::Refused(why) = admit(&attempt, &now) else { panic!("refused") };
    assert!(why.contains("daily"), "{why}");
}

#[test]
fn a_plan_attempt_is_never_refused_and_the_allowance_only_warns() {
    let budget = Budget { allowance: Some(Allowance { tokens: 1_000 }), cap: Some(Cap::Tokens(1)), ..Budget::default() };
    let mut meter = Meter::default();
    meter.record(ProviderId::Claude, "claude-sonnet-5-5", &Usage { input_tokens: 990, ..Usage::default() }, &PriceTable::default());
    let attempt = Attempt { provider: ProviderId::Claude, paid: false, estimate: Estimate { tokens: 100, usd: None }, budget: &budget };
    let Admission::Admitted { warning: Some(warning) } = admit(&attempt, &standing(&meter, &Reserved::default())) else { panic!("admitted with a warning") };
    assert!(warning.contains("soft") && warning.contains("claude"), "{warning}");
    let small = Attempt { estimate: Estimate { tokens: 5, usd: None }, ..attempt };
    assert_eq!(admit(&small, &standing(&meter, &Reserved::default())), Admission::Admitted { warning: None });
    let other_provider = Attempt { provider: ProviderId::Codex, ..attempt };
    assert_eq!(admit(&other_provider, &standing(&meter, &Reserved::default())), Admission::Admitted { warning: None }, "the allowance is per provider");
}

#[test]
fn the_estimate_is_the_prompt_plus_an_output_allowance_priced_when_the_model_has_a_price() {
    let prices = PriceTable::parse("m 1 0.1 4").0;
    let e = Estimate::of("m", 1_000_000, &prices);
    assert_eq!(e.tokens, 1_000_000 + OUTPUT_ALLOWANCE_TOKENS);
    assert!((e.usd.unwrap() - (1.0 + OUTPUT_ALLOWANCE_TOKENS as f64 * 4.0 / 1e6)).abs() < 1e-9);
    assert_eq!(Estimate::of("other", 10, &prices).usd, None);
}
