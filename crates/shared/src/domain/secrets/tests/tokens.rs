use super::*;

// Each fake key is written as prefix + body, so no whole key shape sits in the source
// (a secret scanner would take it for a real one); the test sees the joined string.
macro_rules! prefix_test {
    ($name:ident, $prefix:literal) => {
        #[test]
        fn $name() {
            assert_eq!(redact(concat!("before ", $prefix, "abcdefghijklmnopqrstuvwx", " after")).text, "before [REDACTED:api-key] after");
        }
    };
}

prefix_test!(xai_key, "xai-");
prefix_test!(stripe_live_key, "sk_live_");
prefix_test!(stripe_test_key, "sk_test_");
prefix_test!(stripe_restricted_key, "rk_live_");
prefix_test!(gitlab_key, "glpat-");
prefix_test!(hugging_face_key, "hf_");
prefix_test!(npm_key, "npm_");
prefix_test!(google_oauth_key, "ya29.");

#[test]
fn basic_authorization_is_masked() {
    assert_eq!(redact("Authorization: Basic dXNlcjpwYXNz").text, "Authorization: Basic [REDACTED:bearer-token]");
    assert_eq!(redact("Authorization: Bearer abcdef123456").text, "Authorization: Bearer [REDACTED:bearer-token]");
    assert!(redact("Basic principles are useful").is_clean());
}

#[test]
fn google_key_before_non_key_character_is_masked_without_suffix() {
    let key = "AIzaSyA-abcdefghijklmnopqrstuvwxyz01234";
    assert_eq!(key.len(), 39);
    assert_eq!(redact(&format!("{key}/path")).text, "[REDACTED:google-key]/path");
    assert!(redact(&format!("{key}A")).is_clean());
}

#[test]
fn a_token_after_an_equals_sign_is_blanked_whatever_the_variable_is_called() {
    for text in ["OPENAI_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz0123", "GH_PAT=ghp_abcdefghijklmnopqrstuvwxyz0123456789"] {
        let r = redact(text);
        assert!(!r.text.contains("sk-proj") && !r.text.contains("ghp_"), "{}", r.text);
        assert!(r.text.starts_with(text.split('=').next().unwrap()), "the name stays: {}", r.text);
    }
}

#[test]
fn well_known_token_shapes_are_found() {
    let cases: [(&str, SecretKind); 8] = [
        ("key sk-ant-api03-abcdefghijklmnopqrstuvwx end", SecretKind::AnthropicKey),
        ("sk-proj-abcdefghijklmnopqrstuvwxyz012345", SecretKind::OpenAiStyleKey),
        ("token ghp_abcdefghijklmnopqrstuvwxyz0123456789", SecretKind::GithubToken),
        ("id AKIAABCDEFGHIJKLMNOP.", SecretKind::AwsAccessKey),
        ("xoxb-1234567890-abcdefghij", SecretKind::SlackToken),
        ("AIzaSyA-abcdefghijklmnopqrstuvwxyz01234", SecretKind::GoogleKey),
        ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghij", SecretKind::Jwt),
        ("Authorization: Bearer abcdef0123456789abcdef", SecretKind::BearerToken),
    ];
    for (text, kind) in cases {
        assert_eq!(kinds(text), vec![kind], "{text}");
    }
}
