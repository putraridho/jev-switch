use super::*;
use jev_providers::application::provider_registry::BlockReason;
use crate::render::ProviderStatus;
use super::super::providers::{avail_rows, complete_ctx, status_facts};

#[test]
fn rows_carry_the_probe_and_what_happened_since() {
    let mut rt = runtime();
    rt.cool_down(Codex, Some("1760003600"), NOW);
    rt.blocked.insert(Grok, BlockReason::Auth);
    rt.blocked.insert(Claude, BlockReason::UnusableHere);
    let rows = avail_rows(&rt, NOW);
    assert_eq!(rows.iter().map(|r| r.provider.as_str()).collect::<Vec<_>>(), ["claude", "deepseek", "codex", "gemini", "grok"]);
    assert_eq!(rows[1].status, ProviderStatus::NoKey);
    assert_eq!(rows[2].status, ProviderStatus::Ready);
    assert!(rows[2].cooldown.as_deref().is_some_and(|c| c.contains('h') || c.contains('m')), "{:?}", rows[2].cooldown);
    assert_eq!(rows[3].status, ProviderStatus::NotInstalled);
    assert_eq!(rows[4].status, ProviderStatus::NotLoggedIn);
    assert!(rows[4].detail.contains("/providers refresh"));
    assert_eq!(rows[0].status, ProviderStatus::Unknown, "blocked for this directory");
    assert!(rows[0].detail.contains("/new"));
}

#[test]
fn completion_knows_the_usable_providers_models_and_changed_files() {
    let e = engine(runtime());
    let ctx = complete_ctx(&e, NOW);
    assert_eq!(ctx.ready_providers, ["claude", "codex"]);
    assert!(ctx.models.iter().all(|m| m.starts_with("claude:") || m.starts_with("codex:")), "{:?}", ctx.models);
    assert!(ctx.models.contains(&"codex:gpt-6.1-sol".to_string()));
    assert!(ctx.changed_files.is_empty());
}

#[test]
fn status_lists_ready_providers_and_cooldowns() {
    let mut rt = runtime();
    rt.cool_down(Codex, Some("1760003600"), NOW);
    let e = engine(rt);
    let f = status_facts(&e, None, Some("/home/me"), NOW);
    assert_eq!(f.project, "~/proj");
    assert!(f.routing_on && !f.orchestrate);
    assert_eq!(f.pin, "auto");
    assert_eq!(f.ready.iter().map(|r| r.provider.as_str()).collect::<Vec<_>>(), ["claude"], "codex is cooling down");
    assert_eq!(f.cooldowns.len(), 1);
    assert_eq!(f.cooldowns[0].0, "codex");
    assert_eq!(f.turns, 0);
}
