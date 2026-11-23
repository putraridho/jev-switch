//! What a turn teaches the engine about caches, windows and token rates, and what that does
//! not change: the cache is advisory.

use super::*;
use jev_routing::domain::cache_facts::{ Basis, CacheKey };

fn reported(input: u64, cached: u64, context: Option<u64>) -> Usage {
    Usage {
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: 7,
        context_tokens: context,
        ..Usage::default()
    }
}

#[tokio::test]
async fn only_the_part_the_provider_reported_cached_is_warm_and_the_facts_are_filed_by_model_and_backend() {
    let mut r = rig(&[Claude]);
    *r.usage.lock().unwrap() = Usage {
        cache_written_tokens: Some(2_000),
        ..reported(50_000, 30_000, Some(50_000))
    };
    r.turn(LONG, new_task(TaskType::Feature)).await;
    let warm = r.engine.warm_cache().expect("an answered turn leaves a cache");
    assert_eq!(
        (warm.tokens, warm.tokens_basis),
        (32_000, Basis::Observed),
        "read plus written, not the whole context"
    );
    assert_eq!(warm.model, r.started().last().unwrap().model, "filed under the model that ran");
    let key = warm.key();
    assert_eq!(
        key,
        CacheKey::new(Claude, &warm.model, &r.engine.runtime.fundings.identity(Claude))
    );
    let facts = r.engine.cache_book().get(&key).expect("counted");
    assert_eq!((facts.samples, facts.hit_percent()), (1, Some(60)));
}

#[tokio::test]
async fn a_turn_without_reported_counters_leaves_the_book_alone_and_the_cache_assumed() {
    let mut r = rig(&[Claude]);
    r.turn(LONG, new_task(TaskType::Feature)).await;
    assert!(r.engine.cache_book().is_empty(), "no counters is not a zero counter");
    let warm = r.engine.warm_cache().unwrap();
    assert_eq!(warm.tokens_basis, Basis::Assumed);
}

#[tokio::test]
async fn facts_are_saved_for_the_next_session_without_prompt_text() {
    let dir = jev_testkit::scratch_dir::scratch_dir("engine-cache", "saved");
    let mut r = rig(&[Claude]);
    r.engine = r.engine.with_persistence(Some(dir.clone()));
    *r.usage.lock().unwrap() = reported(10_000, 9_000, Some(10_000));
    let secret = "rotate the credentials of the production database";
    r.turn(secret, new_task(TaskType::Feature)).await;
    let text = std::fs::read_to_string(dir.join(jev_routing::domain::cache_facts::FILE)).unwrap();
    assert!(text.starts_with("claude\t") && text.contains("\t10000\t9000\t1\t"), "{text}");
    assert!(!text.contains("credentials"), "{text}");
    assert_eq!(
        crate::application::cache_store::load(&*r.engine.host, Some(&dir), NOW),
        *r.engine.cache_book()
    );
}

#[tokio::test]
async fn a_window_the_cli_reports_replaces_the_catalogs_only_on_that_backend() {
    let mut r = rig(&[Claude]);
    *r.usage.lock().unwrap() = Usage {
        window_tokens: Some(321_000),
        ..reported(1_000, 0, Some(1_000))
    };
    r.turn(LONG, new_task(TaskType::Feature)).await;
    let ran = r.started().last().unwrap().model.clone();
    let ran = catalog::find(Claude, &ran).unwrap();
    assert_eq!(r.engine.runtime.window_of(ran), 321_000);
    assert_ne!(ran.context_tokens, 321_000);
    // The same model behind another backend does not inherit it.
    r.engine.runtime.fundings = jev_providers::domain::funding::Fundings
        ::default()
        .with(Claude, jev_providers::domain::funding::Funding::Custom);
    assert_eq!(r.engine.runtime.window_of(ran), ran.context_tokens);
}

#[tokio::test]
async fn a_cache_filed_under_another_backend_is_not_warm_for_this_one() {
    let mut r = rig(&[Claude]);
    *r.usage.lock().unwrap() = reported(90_000, 80_000, Some(90_000));
    r.turn(LONG, new_task(TaskType::Feature)).await;
    assert!(r.engine.warm_cache().is_some());
    r.engine.runtime.fundings = jev_providers::domain::funding::Fundings
        ::default()
        .with(Claude, jev_providers::domain::funding::Funding::Custom);
    let before = r.ui.routes.len();
    r.turn(LONG, want(TaskType::Feature)).await;
    assert!(r.ui.routes.len() > before);
    let after = r.engine.warm_cache().unwrap();
    assert_eq!(
        after.backend,
        "custom endpoint (billing unknown)",
        "re-filed under the backend now in force"
    );
    assert_eq!(r.engine.cache_book().iter().count(), 2, "two backends, two caches");
}

#[tokio::test]
async fn the_rate_is_learned_only_from_a_turn_with_a_known_request_size() {
    let mut r = rig(&[Claude]);
    let long = "x".repeat(90_000);
    *r.usage.lock().unwrap() = reported(400_000, 0, None);
    r.turn(&long, new_task(TaskType::Feature)).await;
    assert!(
        r.engine.ratios.get(Claude).is_assumed(),
        "cumulative consumption is not a request size"
    );
    *r.usage.lock().unwrap() = reported(400_000, 0, Some(30_000));
    r.turn(&long, want(TaskType::Feature)).await;
    let rate = r.engine.ratios.get(Claude);
    assert_eq!(rate.samples, 1);
    assert!(rate.milli > 5_000, "the 180k characters of two prompts over 30k tokens: {rate:?}");
    assert!(r.engine.ratios.get(Codex).is_assumed());
}

#[tokio::test]
async fn the_outgoing_prompt_handoff_tools_and_answer_room_decide_which_models_fit() {
    let mut r = rig(&[Claude, Codex]);
    r.script(Codex, OK);
    r.turn(LONG, new_task(TaskType::Feature)).await;
    let ran = r.started().last().unwrap().provider;
    let behind = if ran == Claude { Codex } else { Claude };
    let usable = r.engine.runtime.usable(NOW);
    let reserves = r.engine.outgoing_reserves(&usable, LONG, 490_000);
    assert!(
        reserves[&behind] > reserves[&ran],
        "a provider that must be caught up is sent more: {reserves:?}"
    );
    let terra = catalog::find(Codex, "gpt-6.1-terra").unwrap();
    assert!(usable.fits(terra, 490_000), "the session alone fits 500k");
    let sized = usable.clone().with_reserves(reserves);
    assert!(!sized.fits(terra, 490_000), "a prompt, tools and room for the answer do not");
    assert!(sized.fits(terra, 400_000));
    assert!(
        sized.fits(catalog::find(Claude, MODEL_MID).unwrap(), 490_000),
        "a 1M window still holds it"
    );
}
