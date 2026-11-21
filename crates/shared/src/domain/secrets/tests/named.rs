use super::*;

#[test]
fn hyphenated_api_key_header_is_masked() {
    assert_eq!(redact("x-api-key: hunter2hunter2").text, "x-api-key: [REDACTED:named-secret]");
}

#[test]
fn hyphenated_api_key_is_masked() {
    assert_eq!(redact("api-key = hunter2hunter2").text, "api-key = [REDACTED:named-secret]");
}

#[test]
fn names_ending_in_key_are_masked() {
    for name in ["DEEPSEEK_KEY", "XAI_KEY", "SERVICE-KEY", "stripe_secret_key", "client_key", "LICENSEKEY"] {
        assert_eq!(redact(&format!("{name}=hunter2hunter2")).text, format!("{name}=[REDACTED:named-secret]"), "{name}");
    }
    for text in ["keyboard is useful", "monkey = 3", "XAI_KEY=short", "primary_key: user_id_col", "cache_key = request_path",
        "partition_key: tenant_identifier", "hotkey: ctrl+shift+k", "key: some_value_here", "password: String", "token: string"] {
        assert!(redact(text).is_clean(), "{text}");
    }
}

#[test]
fn a_multibyte_character_before_the_name_does_not_panic() {
    assert!(redact("Keep PKCE – : mandatory").is_clean());
    assert!(redact("Keep PKCE –: mandatory").is_clean());
    assert_eq!(redact("note – password: hunter2hunter2").text, "note – password: [REDACTED:named-secret]");
}

#[test]
fn credentials_name_is_masked() {
    assert_eq!(redact("credentials: hunter2hunter2").text, "credentials: [REDACTED:named-secret]");
}

#[test]
fn escaped_json_password_is_masked() {
    assert_eq!(redact(r#"{\"password\": \"hunter2hunter2\"}"#).text, r#"{\"password\": \"[REDACTED:named-secret]\"}"#);
}

#[test]
fn a_quoted_multi_word_value_is_blanked_whole() {
    let r = redact(r#"{"password": "my long pass phrase", "user": "bob"}"#);
    assert!(!r.text.contains("long") && !r.text.contains("phrase"), "{}", r.text);
    assert!(r.text.contains("bob"), "{}", r.text);
}

#[test]
fn named_secrets_need_a_secret_name_and_a_real_value() {
    assert_eq!(kinds("DB_PASSWORD=hunter2hunter2"), vec![SecretKind::NamedSecret]);
    assert_eq!(kinds(r#"{"api_key": "abcd1234efgh"}"#), vec![SecretKind::NamedSecret]);
    // A reference, a placeholder, a short value and a non-secret name are not secrets.
    assert!(kinds("PASSWORD=$DB_PASSWORD").is_empty());
    assert!(kinds("API_KEY=xxxxxxxxxxxx").is_empty());
    assert!(kinds("token=abc").is_empty());
    assert!(kinds("PATH=/usr/local/bin:/usr/bin").is_empty());
    assert!(kinds("const token = process.env.TOKEN_VALUE;").is_empty());
}
