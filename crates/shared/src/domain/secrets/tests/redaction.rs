use super::*;

#[test]
fn url_passwords_are_masked_without_changing_the_url() {
    assert_eq!(redact("visit https://alice:hunter2@example.com/path and postgres://bob:p@db:5432/x").text,
        "visit https://alice:[REDACTED:named-secret]@example.com/path and postgres://bob:[REDACTED:named-secret]@db:5432/x");
    assert_eq!(redact("https://api-key:password@example.com").text, "https://api-key:[REDACTED:named-secret]@example.com");
    assert!(redact("visit https://example.com/path").is_clean());
}

#[test]
fn private_key_blocks_go_whole_even_when_cut_off() {
    let pem = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nabc\n-----END RSA PRIVATE KEY-----\nafter";
    let r = redact(pem);
    assert_eq!(r.text, "before\n[REDACTED:private-key]\nafter");
    let cut = redact("x\n-----BEGIN PRIVATE KEY-----\nMIIEow");
    assert_eq!(cut.text, "x\n[REDACTED:private-key]");
    // A public key or a certificate is not secret.
    assert!(redact("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----").is_clean());
}

#[test]
fn ordinary_text_and_code_are_untouched() {
    let text = "fn main() { let sk = 3; println!(\"skip-this-one\"); } // see src/token.rs and the AKIA docs";
    assert!(redact(text).is_clean());
    assert_eq!(redact(text).text, text);
    assert!(redact("héllo wörld ✓ 日本語").is_clean());
}

#[test]
fn redaction_keeps_surrounding_text_and_reports_kinds() {
    let r = redact("a sk-ant-api03-abcdefghijklmnopqrstuvwx b DB_PASSWORD=hunter2hunter2 c");
    assert_eq!(r.text, "a [REDACTED:anthropic-key] b DB_PASSWORD=[REDACTED:named-secret] c");
    assert_eq!(r.kinds, vec![SecretKind::AnthropicKey, SecretKind::NamedSecret]);
    // Redacting twice changes nothing more.
    assert!(redact(&r.text).is_clean());
}

#[test]
fn findings_never_carry_the_secret() {
    let f = scan("sk-ant-api03-abcdefghijklmnopqrstuvwx");
    assert_eq!(f.len(), 1);
    assert!(!format!("{f:?}").contains("abcdefgh"));
}
