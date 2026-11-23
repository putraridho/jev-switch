use super::*;
use crate::domain::budget::{Allowance, PriceTable};
use jev_backend::{ProviderId, Usage};

#[test]
fn paid_money_and_subscription_quota_are_shown_apart_with_their_kind_of_limit() {
    let mut meter = Meter::default();
    meter.record(ProviderId::DeepSeek, "deepseek-flash", &Usage { input_tokens: 400, ..Usage::default() }, &PriceTable::default());
    meter.record(ProviderId::Claude, "claude-sonnet-5-5", &Usage { input_tokens: 2_000_000, ..Usage::default() }, &PriceTable::default());
    let reserved = Reserved { attempts: 2, paid_tokens: 16_000, ..Reserved::default() };
    let budget = Budget { cap: Some(Cap::Tokens(100_000)), daily: Some(Cap::Tokens(1_000_000)), allowance: Some(Allowance { tokens: 1_000_000 }) };
    let text = limits_text(&budget, &meter, &reserved, &meter);
    assert!(text.contains("Paid money (hard"), "{text}");
    assert!(text.contains("session:") && text.contains("16k tokens reserved") && text.contains("cap 100k tokens: ok"), "{text}");
    assert!(text.contains("today") && text.contains("cap 1m tokens"), "{text}");
    assert!(text.contains("Subscription quota (soft") && text.contains("claude: 2m used") && text.contains("past the allowance"), "{text}");
    assert!(!text.contains("deepseek: "), "paid use is not subscription quota: {text}");
    assert!(text.contains("In flight: 2 attempt(s)"), "{text}");
}

#[test]
fn with_no_limits_it_says_how_to_set_them() {
    let text = limits_text(&Budget::default(), &Meter::default(), &Reserved::default(), &Meter::default());
    assert!(text.contains("no cap") && text.contains("/budget allowance"), "{text}");
}
