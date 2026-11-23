use super::*;
use crate::domain::model_table::models::by_id;

#[test]
fn pick_only_ever_returns_a_usable_provider() {
    for mask in 0..32u32 {
        let provs = providers_in(mask);
        let usable = Usable::all_of(provs.clone());
        for t in TaskType::ALL {
            for vh in [false, true] {
                let chain = default_chain(t, vh);
                if let Some(p) = pick(&chain, t, &usable, &open(), 0) {
                    assert!(provs.contains(&p.spec.provider), "{t} mask {mask}");
                    assert!(allowed_for(t, p.spec));
                }
            }
        }
    }
}

#[test]
fn nothing_usable_gives_none() {
    let usable = Usable::all_of([]);
    for t in TaskType::ALL {
        assert!(pick(&default_chain(t, true), t, &usable, &open(), 0).is_none());
    }
}

#[test]
fn security_never_picks_deepseek_or_grok() {
    let usable = Usable::all_of([DeepSeek, Grok]);
    for vh in [false, true] {
        let chain = default_chain(TaskType::Security, vh);
        assert!(pick(&chain, TaskType::Security, &usable, &open(), 0).is_none());
    }
    // Even a hand-built chain containing them cannot get through `pick`.
    let sneaky = [by_id("deepseek-v4-pro"), by_id("grok-4.7")];
    assert!(pick(&sneaky, TaskType::Security, &usable, &open(), 0).is_none());
}

#[test]
fn credits_models_are_skipped_unless_allowed() {
    let usable = Usable::all_of([Claude]);
    let chain = default_chain(TaskType::Security, true);
    let no = Allowances { credits: false, per_token: true };
    assert_eq!(pick(&chain, TaskType::Security, &usable, &no, 0).unwrap().spec.id, "claude-opus-5-5");
    let yes = Allowances { credits: true, per_token: false };
    assert_eq!(pick(&chain, TaskType::Security, &usable, &yes, 0).unwrap().spec.id, "claude-fable-5-2");
}

#[test]
fn per_token_models_are_skipped_unless_allowed() {
    let usable = Usable::all_of([DeepSeek]);
    let chain = default_chain(TaskType::SimpleEdit, false);
    let no = Allowances { credits: true, per_token: false };
    assert!(pick(&chain, TaskType::SimpleEdit, &usable, &no, 0).is_none());
    let yes = Allowances { credits: false, per_token: true };
    assert_eq!(pick(&chain, TaskType::SimpleEdit, &usable, &yes, 0).unwrap().spec.id, "deepseek-flash");
}

#[test]
fn a_session_bigger_than_the_window_skips_the_model() {
    let usable = Usable::all_of([Codex, Claude]);
    let chain = default_chain(TaskType::Review, false);
    assert_eq!(pick(&chain, TaskType::Review, &usable, &open(), 400_000).unwrap().spec.id, "gpt-6.1-sol");
    let p = pick(&chain, TaskType::Review, &usable, &open(), 600_000).unwrap();
    assert_eq!(p.spec.id, "claude-opus-5-5");
    assert_eq!(p.rank, 3);
    assert!(pick(&chain, TaskType::Review, &usable, &open(), 2_000_000).is_none());
}

#[test]
fn a_window_the_backend_reported_replaces_the_catalogs_and_survives_filtering() {
    let chain = default_chain(TaskType::Review, false);
    let reported = |tokens| std::collections::HashMap::from([((Codex, "gpt-6.1-sol".to_string()), tokens)]);
    let wide = Usable::all_of([Codex, Claude]).with_windows(reported(900_000));
    assert_eq!(pick(&chain, TaskType::Review, &wide, &open(), 600_000).unwrap().spec.id, "gpt-6.1-sol", "catalog says 500k, the backend says 900k");
    let narrow = Usable::all_of([Codex, Claude]).with_windows(reported(100_000));
    assert_eq!(pick(&chain, TaskType::Review, &Usable::all_of([Codex, Claude]), &open(), 200_000).unwrap().spec.id, "gpt-6.1-sol");
    assert_ne!(pick(&chain, TaskType::Review, &narrow, &open(), 200_000).unwrap().spec.id, "gpt-6.1-sol", "a smaller reported window also counts");
    let kept = wide.filter(|_, _| true).only_providers(&[Codex]).without(&Default::default());
    assert_eq!(pick(&chain, TaskType::Review, &kept, &open(), 600_000).unwrap().spec.id, "gpt-6.1-sol", "filtering keeps the reported windows");
}

#[test]
fn only_claude_hard_debug_picks_opus() {
    let usable = Usable::all_of([Claude]);
    let no_credits = Allowances { credits: false, per_token: false };
    let chain = default_chain(TaskType::HardDebug, false);
    let p = pick(&chain, TaskType::HardDebug, &usable, &no_credits, 0).unwrap();
    assert_eq!(p.spec.id, "claude-opus-5-5");
    assert!(!p.dropped_tier);
    // Very hard with credits on: Fable (Top) is the first usable entry.
    let chain = default_chain(TaskType::HardDebug, true);
    let p = pick(&chain, TaskType::HardDebug, &usable, &open(), 0).unwrap();
    assert_eq!(p.spec.id, "claude-fable-5-2");
}

#[test]
fn dropped_tier_means_below_what_the_task_needs_not_below_the_chain_head() {
    // Explain needs an everyday model: Sonnet is not a drop just because Grok 4.7 heads the chain.
    let chain = default_chain(TaskType::Explain, false);
    let p = pick(&chain, TaskType::Explain, &Usable::all_of([Claude]), &open(), 0).unwrap();
    assert_eq!(p.spec.id, "claude-sonnet-5-5");
    assert!(!p.dropped_tier);
    // A simple edit on a light model is exactly what it needs.
    let chain = default_chain(TaskType::SimpleEdit, false);
    let p = pick(&chain, TaskType::SimpleEdit, &Usable::all_of([Codex]), &open(), 0).unwrap();
    assert!(!p.dropped_tier, "{}", p.spec.id);
    // Explain landing on a light model (only Gemini usable) is a real drop.
    let chain = default_chain(TaskType::Explain, false);
    let p = pick(&chain, TaskType::Explain, &Usable::all_of([Gemini]), &open(), 0).unwrap();
    assert_eq!(p.spec.id, "gemini-3.8-flash");
    assert!(p.dropped_tier);
}
