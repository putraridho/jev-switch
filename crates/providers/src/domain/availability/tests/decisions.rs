//! What each provider's probe decides from the facts it is given.

use crate::domain::availability::claude::{decide_claude, ClaudeLogin};
use crate::domain::availability::codex::{decide_codex, decode_base64url, parse_codex_login};
use crate::domain::availability::deepseek::decide_deepseek;
use crate::domain::availability::gemini::{decide_gemini, GeminiFacts};
use crate::domain::availability::grok::{decide_grok, GrokFacts};
use crate::domain::availability::State;

// ---- Claude

#[test]
fn claude_decisions() {
    let login = |logged_in, plan: Option<&str>| Some(ClaudeLogin { logged_in, plan: plan.map(String::from) });
    assert_eq!(decide_claude(false, login(true, None)).state, State::NotInstalled);
    assert_eq!(decide_claude(true, None).state, State::Unknown);
    assert_eq!(decide_claude(true, login(false, None)).state, State::NotLoggedIn);
    let a = decide_claude(true, login(true, Some("max_5x")));
    assert_eq!((a.state, a.plan.as_deref(), a.verified), (State::Ready, Some("max_5x"), true));
}

// ---- DeepSeek

#[test]
fn deepseek_decisions() {
    assert_eq!(decide_deepseek(false, true).state, State::NotInstalled);
    assert_eq!(decide_deepseek(true, false).state, State::NoKey);
    let a = decide_deepseek(true, true);
    assert_eq!((a.state, a.plan.as_deref()), (State::Ready, Some("pay-per-token")));
}

// ---- Codex

#[test]
fn codex_login_text() {
    assert_eq!(parse_codex_login(true, "Logged in using ChatGPT\n"), Some(true));
    assert_eq!(parse_codex_login(false, "Not logged in"), Some(false));
    assert_eq!(parse_codex_login(true, "Not logged in"), Some(false));
    assert_eq!(parse_codex_login(false, ""), Some(false));
    assert_eq!(parse_codex_login(true, "???"), None);
}

#[test]
fn base64url_decoding() {
    assert_eq!(decode_base64url("aGVsbG8").unwrap(), b"hello");
    assert_eq!(decode_base64url("aGVsbG8=").unwrap(), b"hello");
    assert_eq!(decode_base64url("Pz8_Pz8-").unwrap(), b"?????>");
    assert_eq!(decode_base64url("a$b"), None);
    assert_eq!(decode_base64url("").unwrap(), b"");
}


#[test]
fn codex_decisions() {
    let models = || Some(vec!["m1".to_string()]);
    assert_eq!(decide_codex(false, Some(true), None, None).state, State::NotInstalled);
    assert_eq!(decide_codex(true, None, None, None).state, State::Unknown);
    assert_eq!(decide_codex(true, Some(false), Some("plus"), models()).state, State::NotLoggedIn);
    let a = decide_codex(true, Some(true), Some("free"), models());
    assert_eq!((a.state, a.plan.as_deref(), a.models), (State::Ready, Some("free"), models()));
    assert_eq!(decide_codex(true, Some(true), None, Some(vec![])).models, None);
}

// ---- Gemini

#[test]
fn gemini_decisions() {
    let installed = GeminiFacts { gemini_installed: true, ..Default::default() };
    assert_eq!(decide_gemini(GeminiFacts::default()).state, State::NotInstalled);
    assert_eq!(decide_gemini(installed).state, State::NotLoggedIn);
    for configured in [
        GeminiFacts { api_key_env: true, ..installed },
        GeminiFacts { settings_auth: true, ..installed },
        GeminiFacts { oauth_creds: true, ..installed },
        GeminiFacts { agy_installed: true, gemini_installed: false, agy_creds: true, ..Default::default() },
    ] {
        let a = decide_gemini(configured);
        assert_eq!((a.state, a.verified), (State::Ready, false));
    }
    // A login with no CLI to use it is not usable.
    assert_eq!(decide_gemini(GeminiFacts { oauth_creds: true, ..Default::default() }).state, State::NotInstalled);
}

// ---- Grok

#[test]
fn grok_decisions() {
    assert_eq!(decide_grok(GrokFacts::default()).state, State::NotInstalled);
    assert_eq!(decide_grok(GrokFacts { real_binary: false, home_dir: true, api_key: true }).state, State::NotInstalled);
    assert_eq!(decide_grok(GrokFacts { real_binary: true, ..Default::default() }).state, State::NotLoggedIn);
    let a = decide_grok(GrokFacts { real_binary: true, home_dir: true, ..Default::default() });
    assert_eq!((a.state, a.verified), (State::Ready, false));
    // An API key alone is enough: no `grok login`, no ~/.grok.
    let k = decide_grok(GrokFacts { real_binary: true, api_key: true, ..Default::default() });
    assert_eq!((k.state, k.verified), (State::Ready, false));
    assert!(k.detail.contains("XAI_API_KEY"), "{}", k.detail);
}
