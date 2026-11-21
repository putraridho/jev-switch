use super::*;
use startup_hint::first_run_hint;

#[test]
fn hint_is_absent_when_two_are_ready_and_nothing_is_half_configured() {
    use State::*;
    for states in [
        [Ready, NotInstalled, Ready, NotInstalled, NotInstalled],
        [Ready, NoKey, Ready, NotInstalled, NotInstalled],
        [Ready, Ready, Unknown, NotInstalled, NotInstalled],
        [Ready; 5],
    ] {
        assert_eq!(first_run_hint(&all(states)), None, "{states:?}");
    }
}

#[test]
fn hint_names_a_half_configured_provider_even_with_two_ready() {
    use State::*;
    let hint = first_run_hint(&all([Ready, NotInstalled, Ready, NotLoggedIn, NotInstalled])).unwrap();
    assert!(hint.contains("gemini is installed but not logged in"), "{hint}");
    assert!(!hint.contains("nothing to route between"));
}

#[test]
fn hint_with_one_ready_says_so_and_never_nags_about_uninstalled_providers() {
    use State::*;
    let hint = first_run_hint(&all([Ready, NotInstalled, NotInstalled, NotInstalled, NotInstalled])).unwrap();
    assert!(hint.contains("nothing to route between"));
    for name in ["codex", "gemini", "grok", "deepseek"] {
        assert!(!hint.contains(name), "{hint}");
    }
}

#[test]
fn hint_gives_the_exact_fix_for_each_login() {
    use State::*;
    let hint = first_run_hint(&all([NotLoggedIn, NoKey, NotLoggedIn, NotInstalled, NotInstalled])).unwrap();
    assert!(hint.contains("claude is installed but not logged in: claude auth login"), "{hint}");
    assert!(hint.contains("codex is installed but not logged in: codex login"), "{hint}");
    assert!(hint.contains("deepseek has no API key: Add DEEPSEEK_API_KEY=<key> to ~/.jev-router.env"), "{hint}");
    assert!(hint.starts_with("No provider is ready yet"));
}

#[test]
fn hint_with_nothing_installed_points_at_doctor() {
    let hint = first_run_hint(&all([State::NotInstalled; 5])).unwrap();
    assert!(hint.contains("No provider is ready") && hint.contains("doctor"), "{hint}");
}

#[test]
fn deepseek_without_a_key_is_not_nagged_when_two_others_are_ready() {
    let hint = first_run_hint(&all([State::Ready, State::NoKey, State::Ready, State::NotInstalled, State::NotInstalled]));
    assert_eq!(hint, None);
}
