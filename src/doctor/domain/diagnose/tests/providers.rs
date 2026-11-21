use super::*;
use crate::providers::domain::login_hints::{login_command, login_verified};
use crate::shared::ALL_PROVIDERS;
use provider_checks::provider_id;

#[test]
fn provider_state_table_gives_severity_and_exact_first_fix() {
    use State::*;
    // (provider, state, severity, a step that must be present verbatim)
    let table: &[(ProviderId, State, Severity, &str)] = &[
        (Claude, Ready, Severity::Ok, ""),
        (Claude, NotLoggedIn, Severity::Warn, "claude auth login"),
        (Claude, NotInstalled, Severity::Info, "Install Claude Code: https://code.claude.com/docs/en/quickstart"),
        (Claude, Unknown, Severity::Warn, "claude auth status"),
        (Codex, Ready, Severity::Ok, ""),
        (Codex, NotLoggedIn, Severity::Warn, "codex login"),
        (Codex, NotLoggedIn, Severity::Warn, "codex login --device-auth (if this machine is headless)"),
        (Codex, NotInstalled, Severity::Info, "npm i -g @openai/codex (or see the vendor's install page)"),
        (Codex, Unknown, Severity::Warn, "codex login status"),
        (Gemini, Ready, Severity::Ok, ""),
        (Gemini, NotLoggedIn, Severity::Warn, "Run `gemini` once and choose a login"),
        (Gemini, NotInstalled, Severity::Info, "npm i -g @google/gemini-cli"),
        (Grok, Ready, Severity::Ok, ""),
        (Grok, NotLoggedIn, Severity::Warn, "grok (first run installs it, which can take minutes)"),
        (Grok, NotInstalled, Severity::Info, "Install the Grok CLI"),
        (DeepSeek, Ready, Severity::Ok, ""),
        (DeepSeek, NoKey, Severity::Warn, "Add DEEPSEEK_API_KEY=<key> to ~/.jev-router.env (billed per token)"),
        (DeepSeek, NotInstalled, Severity::Info, "Install Claude Code: https://code.claude.com/docs/en/quickstart"),
    ];
    for (provider, state, severity, step) in table {
        let r = diagnose(&inputs(vec![av(*provider, *state)]));
        let c = find(&r, provider_id(*provider));
        assert_eq!(c.severity, *severity, "{provider} {state:?}");
        if step.is_empty() {
            assert!(c.fix.is_empty(), "{provider} ready needs no fix");
        } else {
            assert!(c.fix.iter().any(|s| s.starts_with(step)), "{provider} {state:?} fix {:?} lacks {step}", c.fix);
        }
    }
}

#[test]
fn every_provider_state_combination_yields_one_check_without_panicking() {
    use State::*;
    for provider in ALL_PROVIDERS {
        for state in [Ready, NotInstalled, NotLoggedIn, NoKey, Unknown] {
            let r = diagnose(&inputs(vec![av(provider, state)]));
            assert_eq!(r.checks.iter().filter(|c| c.id == provider_id(provider)).count(), 1);
        }
    }
}

#[test]
fn not_installed_gives_install_then_login_steps() {
    let r = diagnose(&inputs(vec![av(Codex, State::NotInstalled)]));
    let fix = &find(&r, "provider-codex").fix;
    assert!(fix[0].starts_with("npm i -g @openai/codex"));
    assert_eq!(fix[1], "codex login");
}

#[test]
fn gemini_fix_offers_key_or_login_and_flags_agy_unverified() {
    let r = diagnose(&inputs(vec![av(Gemini, State::NotLoggedIn)]));
    let fix = find(&r, "provider-gemini").fix.join("\n");
    assert!(fix.contains("GEMINI_API_KEY=<key> in ~/.jev-router.env"));
    assert!(fix.contains("unverified: Antigravity users run `agy` and sign in"));
}

#[test]
fn grok_fix_mentions_the_slow_first_run_and_the_plan_caveat() {
    let r = diagnose(&inputs(vec![av(Grok, State::NotLoggedIn)]));
    let fix = find(&r, "provider-grok").fix.join("\n");
    assert!(fix.contains("which can take minutes"));
    assert!(fix.contains("SuperGrok or X Premium+"));
    assert!(fix.contains("unverified"));
}

#[test]
fn deepseek_says_per_token_and_no_subscription() {
    let r = diagnose(&inputs(vec![av(DeepSeek, State::NoKey)]));
    let c = find(&r, "provider-deepseek");
    assert!(c.detail.contains("billed per token"));
    assert!(c.detail.contains("no subscription"));
}

#[test]
fn unverified_providers_are_labelled_and_verified_ones_are_not() {
    for p in ALL_PROVIDERS {
        let r = diagnose(&inputs(vec![av(p, State::Ready)]));
        let labelled = find(&r, provider_id(p)).detail.contains("unverified");
        assert_eq!(labelled, matches!(p, Gemini | Grok), "{p}");
    }
}

#[test]
fn ready_detail_carries_plan_and_models() {
    let a = Availability { plan: Some("plus".into()), models: Some(vec!["m1".into(), "m2".into()]), ..av(Codex, State::Ready) };
    let r = diagnose(&inputs(vec![a]));
    let d = &find(&r, "provider-codex").detail;
    assert!(d.contains("plan: plus") && d.contains("models: m1, m2"), "{d}");
}

#[test]
fn env_path_override_shows_up_in_key_fixes() {
    let mut i = inputs(vec![av(DeepSeek, State::NoKey)]);
    i.env_file = Some(EnvFileFacts { path: "/x/keys.env".into(), mode_bits: Some(0o600), has_keys: true });
    let r = diagnose(&i);
    assert!(find(&r, "provider-deepseek").fix[0].contains("/x/keys.env"));
}

#[test]
fn login_commands_have_one_source_and_the_verified_ones_are_named() {
    assert_eq!(login_command(Claude), Some("claude auth login"));
    assert_eq!(login_command(Codex), Some("codex login"));
    assert_eq!(login_command(Gemini), Some("gemini"));
    assert_eq!(login_command(Grok), Some("grok"));
    assert_eq!(login_command(DeepSeek), None);
    for p in ALL_PROVIDERS {
        assert_eq!(login_verified(p), matches!(p, Claude | Codex), "{p}");
    }
}
