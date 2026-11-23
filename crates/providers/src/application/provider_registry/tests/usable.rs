//! `Runtime::usable`: which models routing may pick.

use super::*;

    #[test]
    fn usable_has_ready_providers_only() {
        let u = runtime().usable(NOW);
        assert!(u.contains(some_model(Claude)));
        assert!(u.contains(some_model(Codex)));
        assert!(!u.contains(some_model(Gemini)));
        assert!(!u.contains(some_model(DeepSeek)));
    }

    #[test]
    fn usable_excludes_cooling_provider_until_it_resets() {
        let mut rt = runtime();
        rt.cooldowns.start_in(Claude, None, NOW, &|_, _| None, &|_| false);
        assert!(!rt.usable(NOW + 1).contains(some_model(Claude)));
        assert!(rt.usable(NOW + 1).contains(some_model(Codex)));
        // Default cooldown is an hour; after that the provider is back.
        assert!(rt.usable(NOW + 2 * 60 * MINUTE_MS).contains(some_model(Claude)));
    }

    #[test]
    fn usable_excludes_a_refused_model_but_not_its_siblings() {
        let mut rt = runtime();
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        assert!(claude.len() >= 2, "test needs two Claude models");
        rt.mark_unusable(Claude, claude[0].id);
        let u = rt.usable(NOW);
        assert!(!u.contains(claude[0]));
        assert!(u.contains(claude[1]));
    }

    #[test]
    fn login_model_list_narrows_usable() {
        let mut a = availability(Claude, State::Ready);
        let claude: Vec<_> = catalog::models().iter().filter(|m| m.provider == Claude).collect();
        a.models = Some(vec![claude[0].id.to_string()]);
        let u = Runtime::for_tests(vec![a]).usable(NOW);
        assert!(u.contains(claude[0]));
        assert!(!u.contains(claude[1]));
    }

    #[test]
    fn usable_carries_the_windows_the_backend_reported() {
        let mut rt = runtime();
        let codex = some_model(Codex);
        assert_eq!(rt.usable(NOW).window_of(codex), codex.context_tokens);
        rt.observe_window(Codex, codex.id, 123_456);
        assert_eq!(rt.usable(NOW).window_of(codex), 123_456);
        rt.blocked.insert(Claude, super::super::BlockReason::Auth);
        assert_eq!(rt.usable(NOW).window_of(codex), 123_456, "also when another provider is skipped");
    }

#[test]
fn a_window_learned_on_one_backend_is_not_applied_to_another() {
    use crate::domain::funding::{ Fundings, Funding };
    let mut rt = runtime();
    let codex = some_model(Codex);
    rt.fundings = Fundings::default().with(Codex, Funding::Custom);
    rt.observe_window(Codex, codex.id, 64_000);
    assert_eq!(rt.usable(NOW).window_of(codex), 64_000);
    rt.fundings = Fundings::default().with(Codex, Funding::Login);
    assert_eq!(
        rt.usable(NOW).window_of(codex),
        codex.context_tokens,
        "the native login keeps the catalog's size"
    );
    assert_eq!(rt.window_of(codex), codex.context_tokens);
    rt.observe_window(Codex, codex.id, 0);
    rt.observe_window(Codex, codex.id, u64::MAX);
    assert_eq!(rt.window_of(codex), codex.context_tokens, "absurd reports are ignored");
}
