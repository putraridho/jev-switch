//! What each provider's probe decides from the facts it is given.

use crate::providers::domain::availability::claude::{decide_claude, parse_claude_login, ClaudeLogin};
use crate::providers::domain::availability::codex::{codex_plan_from_auth, decide_codex, decode_base64url, parse_codex_login};
use crate::providers::domain::availability::deepseek::decide_deepseek;
use crate::providers::domain::availability::gemini::{decide_gemini, GeminiFacts};
use crate::providers::domain::availability::grok::{decide_grok, GrokFacts};
use crate::providers::domain::availability::State;

// ---- Claude

#[test]
fn claude_json_status_logged_in_with_plan() {
    let l = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"max","planTier":"max_5x"}"#).unwrap();
    assert_eq!(l, ClaudeLogin { logged_in: true, plan: Some("max_5x".into()) });
    let l = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"pro"}"#).unwrap();
    assert_eq!(l.plan.as_deref(), Some("pro"));
}

#[test]
fn claude_json_status_logged_out() {
    let l = parse_claude_login("{\n  \"loggedIn\": false\n}\n").unwrap();
    assert!(!l.logged_in);
}

#[test]
fn claude_text_status_is_understood() {
    assert!(!parse_claude_login("Not logged in").unwrap().logged_in);
    assert!(parse_claude_login("Logged in as someone").unwrap().logged_in);
    assert_eq!(parse_claude_login("something else"), None);
    assert_eq!(parse_claude_login("{\"other\":1}"), None);
}

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

/// Builds a fake auth.json whose JWT payload is `claims` (synthetic, never real).
fn fake_auth(claims: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut payload = String::new();
    for chunk in claims.as_bytes().chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (16 - 8 * i));
        let chars = chunk.len() + 1;
        for i in 0..chars {
            payload.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    format!(r#"{{"tokens":{{"id_token":"head.{payload}.sig"}}}}"#)
}

#[test]
fn codex_plan_comes_from_the_jwt_claim() {
    let auth = fake_auth(r#"{"https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#);
    assert_eq!(codex_plan_from_auth(&auth).as_deref(), Some("plus"));
}

#[test]
fn codex_plan_is_none_when_absent_or_malformed() {
    assert_eq!(codex_plan_from_auth(&fake_auth(r#"{"sub":"x"}"#)), None);
    assert_eq!(codex_plan_from_auth(r#"{"tokens":{"id_token":"nodots"}}"#), None);
    assert_eq!(codex_plan_from_auth(r#"{"tokens":{}}"#), None);
    assert_eq!(codex_plan_from_auth("not json"), None);
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
