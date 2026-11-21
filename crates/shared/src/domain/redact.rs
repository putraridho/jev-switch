//! Secret redaction for anything jev-code writes outside the session: exports, the audit log
//! and shared transcripts. Pure text in, text out, so every rule is a unit test.
//!
//! Three kinds of secret are recognised, none of them by guessing at entropy:
//! well-known token shapes (`sk-...`, `ghp_...`, `AKIA...`, JWTs), the value of a setting
//! whose name says it is secret (`API_KEY=...`, `"password": "..."`), and `Bearer` tokens.
//! PEM private-key blocks are dropped whole.

/// What a secret is replaced by.
pub const MASK: &str = "[REDACTED]";

/// `text` with every secret masked. Idempotent: redacting redacted text changes nothing.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for finding in crate::domain::secrets::scan(text) {
        out.push_str(&text[at..finding.range.start]);
        out.push_str(MASK);
        if finding.kind == crate::domain::secrets::SecretKind::PrivateKey {
            out.push_str(" (private key)");
        }
        at = finding.range.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_token_shapes_are_masked() {
        for secret in [
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "AKIAABCDEFGHIJKLMNOP",
            "xoxb-1234567890-abcdefghij",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
        ] {
            assert_eq!(redact(&format!("use {secret} now")), format!("use {MASK} now"), "{secret}");
        }
    }

    #[test]
    fn shared_detection_covers_additional_tokens_and_spaced_assignments() {
        for secret in [
            "ASIAABCDEFGHIJKLMNOP",
            "ghr_abcdefghijklmnopqrstuvwxyz0123456789",
            "xoxa-1234567890-abcdefghij",
            "xoxr-1234567890-abcdefghij",
            "xoxs-1234567890-abcdefghij",
        ] {
            assert_eq!(redact(&format!("use {secret} now")), format!("use {MASK} now"), "{secret}");
        }
        assert_eq!(redact("SECRET = hunter2hunter2"), format!("SECRET = {MASK}"));
    }

    #[test]
    fn ordinary_words_and_short_values_stay() {
        for plain in ["the sk-learn package", "task-runner", "token=1", "max_tokens: 4096", "max_tokens: 100000", "token_count=123456", "tokens_used: 1500000","key=a", "see AKIA docs"] {
            assert_eq!(redact(plain), plain, "{plain}");
        }
    }

    #[test]
    fn a_numeric_token_credential_is_still_masked() {
        assert_eq!(redact("token=123456"), format!("token={MASK}"));
        assert_eq!(redact("access_tokens: abcdef123456"), format!("access_tokens: {MASK}"));
    }

    #[test]
    fn a_value_after_a_secret_name_is_masked_in_each_spelling() {
        assert_eq!(redact("ANTHROPIC_API_KEY=hunter2hunter2"), format!("ANTHROPIC_API_KEY={MASK}"));
        assert_eq!(redact("export DB_PASSWORD='p4ssw0rd!'"), format!("export DB_PASSWORD='{MASK}'"));
        assert_eq!(redact(r#"{"client_secret": "abc123def456", "ok": 1}"#), format!(r#"{{"client_secret": "{MASK}", "ok": 1}}"#));
        assert_eq!(redact("password: swordfish"), format!("password: {MASK}"));
    }

    #[test]
    fn bearer_tokens_are_masked() {
        assert_eq!(redact("Authorization: Bearer abcdef123456.xyz"), format!("Authorization: Bearer {MASK}"));
        assert_eq!(redact("curl -H 'authorization: bearer abcdef123456'"), format!("curl -H 'authorization: bearer {MASK}'"));
    }

    #[test]
    fn a_private_key_block_is_dropped_whole() {
        let text = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nxyz\n-----END RSA PRIVATE KEY-----\nafter\n";
        assert_eq!(redact(text), format!("before\n{MASK} (private key)\nafter\n"));
    }

    #[test]
    fn redacting_twice_changes_nothing_and_multibyte_text_survives() {
        let text = "tok=sk-ant-api03-abcdefghijklmnopqrstuvwxyz and SECRET=abcdefgh, café ✓";
        let once = redact(text);
        assert_eq!(redact(&once), once);
        assert!(once.ends_with("café ✓"), "{once}");
        assert!(!once.contains("abcdefgh"));
    }
}
