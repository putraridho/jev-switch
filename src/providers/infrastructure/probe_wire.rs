//! Decoding provider CLI and credential output into probe facts.

use serde_json::Value;

use crate::providers::domain::availability::claude::ClaudeLogin;
use crate::providers::domain::availability::codex::decode_base64url;

pub(super) fn parse_claude_login(text: &str) -> Option<ClaudeLogin> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        let logged_in = value.get("loggedIn")?.as_bool()?;
        let plan = ["planTier", "subscriptionType"]
            .iter()
            .find_map(|key| value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()))
            .map(str::to_string);
        return Some(ClaudeLogin { logged_in, plan });
    }
    let lower = text.to_lowercase();
    if lower.contains("not logged in") || lower.contains("not authenticated") {
        Some(ClaudeLogin { logged_in: false, plan: None })
    } else if lower.contains("logged in") {
        Some(ClaudeLogin { logged_in: true, plan: None })
    } else {
        None
    }
}

pub(super) fn codex_plan_from_auth(auth_json: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_json).ok()?;
    let payload = auth.pointer("/tokens/id_token")?.as_str()?.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&decode_base64url(payload)?).ok()?;
    let plan = claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")?.as_str()?;
    (!plan.is_empty()).then(|| plan.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_json_status_logged_in_with_plan() {
        let login = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"max","planTier":"max_5x"}"#).unwrap();
        assert_eq!(login, ClaudeLogin { logged_in: true, plan: Some("max_5x".into()) });
        let login = parse_claude_login(r#"{"loggedIn":true,"subscriptionType":"pro"}"#).unwrap();
        assert_eq!(login.plan.as_deref(), Some("pro"));
    }

    #[test]
    fn claude_json_status_logged_out() {
        let login = parse_claude_login("{\n  \"loggedIn\": false\n}\n").unwrap();
        assert!(!login.logged_in);
    }

    #[test]
    fn claude_text_status_is_understood() {
        assert!(!parse_claude_login("Not logged in").unwrap().logged_in);
        assert!(parse_claude_login("Logged in as someone").unwrap().logged_in);
        assert_eq!(parse_claude_login("something else"), None);
        assert_eq!(parse_claude_login("{\"other\":1}"), None);
    }

    fn fake_auth(claims: &str) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut payload = String::new();
        for chunk in claims.as_bytes().chunks(3) {
            let number = chunk.iter().enumerate().fold(0u32, |acc, (index, byte)| acc | (*byte as u32) << (16 - 8 * index));
            for index in 0..chunk.len() + 1 {
                payload.push(ALPHABET[((number >> (18 - 6 * index)) & 63) as usize] as char);
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
}
