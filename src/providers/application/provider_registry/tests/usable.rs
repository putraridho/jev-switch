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
