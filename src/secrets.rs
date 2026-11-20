//! Finding secrets in text that is about to leave the machine, and blanking them out.
//!
//! The rules are deliberately narrow: well-known token shapes (a recognisable prefix and a
//! plausible length), private key blocks, and `NAME=value` pairs whose NAME says it is a
//! secret. A false positive costs the model one blanked word; a miss leaks a key, so the
//! shapes err toward matching, but plain prose and short words never match.
//!
//! Pure functions only: no IO, no regex crate. Matched text is never stored in a
//! [`Finding`], only where it was and what kind it is, so findings are safe to log.

use std::fmt;
use std::ops::Range;

/// What a secret looks like it is. Printed in the blanked text and in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretKind {
    PrivateKey,
    AnthropicKey,
    OpenAiStyleKey,
    GithubToken,
    AwsAccessKey,
    SlackToken,
    GoogleKey,
    Jwt,
    BearerToken,
    /// `PASSWORD=...`, `"api_key": "..."` and the like.
    NamedSecret,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrivateKey => "private-key",
            Self::AnthropicKey => "anthropic-key",
            Self::OpenAiStyleKey => "api-key",
            Self::GithubToken => "github-token",
            Self::AwsAccessKey => "aws-access-key",
            Self::SlackToken => "slack-token",
            Self::GoogleKey => "google-key",
            Self::Jwt => "jwt",
            Self::BearerToken => "bearer-token",
            Self::NamedSecret => "named-secret",
        }
    }
}

impl fmt::Display for SecretKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One secret found: what kind, and the byte range of it in the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: SecretKind,
    pub range: Range<usize>,
}

/// Text with every secret replaced, and what was replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    pub kinds: Vec<SecretKind>,
}

impl Redacted {
    pub fn is_clean(&self) -> bool {
        self.kinds.is_empty()
    }
}

const PRIVATE_KEY_BEGIN: &str = "-----BEGIN ";
const PRIVATE_KEY_MARK: &str = "PRIVATE KEY-----";
const PRIVATE_KEY_END: &str = "-----END ";

/// Words in a variable name that say its value is a secret.
const SECRET_NAME_WORDS: [&str; 10] = ["secret", "token", "password", "passwd", "api_key", "api-key", "apikey", "private_key", "access_key", "credentials"];
/// A value shorter than this is a flag, a placeholder or a type name (`password: String`),
/// not a credential.
const MIN_SECRET_VALUE: usize = 8;
/// The shortest value masked after a secret name when it has a digit in it.
const SHORT_SECRET_VALUE: usize = 6;

/// A name ending in "key" that holds a credential: an environment-style name in capitals
/// (`DEEPSEEK_KEY`, `XAI_KEY`), or one that says what kind of key (`stripe_secret_key`).
/// Code's own keys (`primary_key`, `cache_key`, `hotkey`, `monkey`) are not secrets.
fn key_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with("_key") || lower.ends_with("-key") || lower.ends_with("key") && name.len() > 3) {
        return false;
    }
    let env_style = name.chars().any(|c| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-'));
    let says_credential = ["api", "secret", "access", "private", "auth", "client", "signing", "master", "license"].iter().any(|w| lower.contains(w));
    env_style && (lower.ends_with("_key") || lower.ends_with("-key") || says_credential) || says_credential
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

/// Every secret in `text`, in order, without overlaps.
pub fn scan(text: &str) -> Vec<Finding> {
    let mut found = private_keys(text);
    found.extend(tokens(text));
    let urls = url_passwords(text);
    found.extend(named_secrets(text, &urls));
    found.extend(urls);
    found.sort_by_key(|f| (f.range.start, std::cmp::Reverse(f.range.end)));
    let mut kept: Vec<Finding> = Vec::new();
    for f in found {
        if kept.last().is_some_and(|k| f.range.start < k.range.end) {
            continue;
        }
        kept.push(f);
    }
    kept
}

/// `text` with each secret replaced by `[REDACTED:<kind>]`.
pub fn redact(text: &str) -> Redacted {
    let findings = scan(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in &findings {
        out.push_str(&text[at..f.range.start]);
        out.push_str(&format!("[REDACTED:{}]", f.kind));
        at = f.range.end;
    }
    out.push_str(&text[at..]);
    Redacted { text: out, kinds: findings.iter().map(|f| f.kind).collect() }
}

/// `-----BEGIN ... PRIVATE KEY-----` up to its END line (or the end of the text when the
/// block is cut off: half a key is still a key).
fn private_keys(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find(PRIVATE_KEY_BEGIN) {
        let start = from + i;
        let label_from = start + PRIVATE_KEY_BEGIN.len();
        let header_end = text[label_from..].find("-----").map_or(text.len(), |e| label_from + e + 5);
        let header = &text[start..header_end];
        if !header.contains(PRIVATE_KEY_MARK) {
            from = start + PRIVATE_KEY_BEGIN.len();
            continue;
        }
        let end = match text[start..].find(PRIVATE_KEY_END) {
            Some(e) => {
                let after = start + e + PRIVATE_KEY_END.len();
                text[after..].find("-----").map_or(text.len(), |d| after + d + 5)
            }
            None => text.len(),
        };
        out.push(Finding { kind: SecretKind::PrivateKey, range: start..end });
        from = end;
    }
    out
}

/// Prefix-shaped tokens, and the token after `Bearer` or `Basic`.
fn tokens(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut previous_word: Option<(usize, usize)> = None;
    let flush = |from: usize, to: usize, out: &mut Vec<Finding>, prev: &mut Option<(usize, usize)>| {
        // A sentence's full stop is not part of the token.
        let to = from + text[from..to].trim_end_matches('.').len();
        let token = &text[from..to];
        let after_auth = prev.is_some_and(|(a, b)| {
            let basic_header = text[a..b].eq_ignore_ascii_case("basic")
                && text[..a].trim_end().strip_suffix(':').is_some_and(|head| name_before(head).eq_ignore_ascii_case("authorization"));
            (text[a..b].eq_ignore_ascii_case("bearer") || basic_header)
                && !text[b..from].is_empty() && text[b..from].chars().all(|c| matches!(c, ' ' | '\t'))
        });
        // `=` is a token character (base64 padding), so `NAME=sk-...` is one word: look
        // behind the first `=` as well.
        let kind_at = token_kind(token).map(|(k, len)| (k, from, len)).or_else(|| {
            let eq = token.find('=')?;
            token_kind(&token[eq + 1..]).map(|(k, len)| (k, from + eq + 1, len))
        });
        if let Some((kind, begin, len)) = kind_at {
            out.push(Finding { kind, range: begin..begin + len });
        } else if after_auth && token.len() >= MIN_SECRET_VALUE && looks_like_a_literal(token) {
            out.push(Finding { kind: SecretKind::BearerToken, range: from..to });
        }
        *prev = Some((from, to));
    };
    for (i, c) in text.char_indices() {
        match (is_token_char(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                flush(s, i, &mut out, &mut previous_word);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        flush(s, text.len(), &mut out, &mut previous_word);
    }
    out
}

fn token_kind(t: &str) -> Option<(SecretKind, usize)> {
    let len = t.len();
    let tail_is_key = |prefix: &str, min: usize| t.starts_with(prefix) && len >= min;
    let kind = if tail_is_key("sk-ant-", 20) {
        Some(SecretKind::AnthropicKey)
    } else if tail_is_key("sk-", 19)
        || ["xai-", "sk_live_", "sk_test_", "rk_live_", "glpat-", "hf_", "npm_", "ya29."].iter().any(|p| tail_is_key(p, 20))
    {
        Some(SecretKind::OpenAiStyleKey)
    } else if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"].iter().any(|p| tail_is_key(p, 24)) || tail_is_key("github_pat_", 30) {
        Some(SecretKind::GithubToken)
    } else if (t.starts_with("AKIA") || t.starts_with("ASIA"))
        && len == 20
        && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        Some(SecretKind::AwsAccessKey)
    } else if t.starts_with("xox") && t[3..].starts_with(['b', 'a', 'p', 'r', 's']) && t[4..].starts_with('-') && len >= 15 {
        Some(SecretKind::SlackToken)
    } else if t.starts_with("AIza") && len >= 39
        && t.as_bytes()[..39].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        && t.as_bytes().get(39).is_none_or(|c| !c.is_ascii_alphanumeric() && !matches!(c, b'_' | b'-'))
    {
        return Some((SecretKind::GoogleKey, 39));
    } else if is_jwt(t) {
        Some(SecretKind::Jwt)
    } else {
        None
    };
    kind.map(|kind| (kind, len))
}

/// Passwords in URL userinfo, without the surrounding URL.
fn url_passwords(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("://") {
        let scheme = text[..at].rsplit(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.')).next().unwrap_or("");
        if !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let start = at + 3;
        let end = text[start..].find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '"' | '\'' | '<' | '>')).map_or(text.len(), |offset| start + offset);
        let authority = &text[start..end];
        if let Some((userinfo, host)) = authority.rsplit_once('@') {
            if !host.is_empty() {
                if let Some((user, password)) = userinfo.split_once(':') {
                    if !user.is_empty() && !password.is_empty() {
                        let from = start + user.len() + 1;
                        out.push(Finding { kind: SecretKind::NamedSecret, range: from..from + password.len() });
                    }
                }
            }
        }
    }
    out
}

fn is_jwt(t: &str) -> bool {
    t.starts_with("eyJ") && t.len() >= 30 && {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
    }
}

/// `NAME=value`, `NAME: value`, `"name": "value"` where NAME says it holds a secret.
fn named_secrets(text: &str, urls: &[Finding]) -> Vec<Finding> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if c != '=' && c != ':' {
            continue;
        }
        if c == ':' && urls.iter().any(|url| url.range.start == i + 1) {
            continue;
        }
        let name = name_before(&text[..i]);
        let lower = name.to_ascii_lowercase();
        if name.is_empty() || !(key_name(name) || SECRET_NAME_WORDS.iter().any(|w| lower.contains(w))) {
            continue;
        }
        let mut v = i + 1;
        let mut quote = None;
        while v < bytes.len() && matches!(bytes[v], b' ' | b'\t') {
            v += 1;
        }
        let escaped_quote = bytes.get(v..v + 2) == Some(b"\\\"");
        if escaped_quote {
            v += 2;
            quote = Some('"');
        }
        while v < bytes.len() && matches!(bytes[v], b' ' | b'\t' | b'"' | b'\'') {
            if bytes[v] == b'"' || bytes[v] == b'\'' {
                quote = Some(bytes[v] as char);
            }
            v += 1;
        }
        // A quoted value runs to its closing quote on the same line, spaces and all.
        let quoted_end = if escaped_quote {
            text[v..].find("\\\"").filter(|e| !text[v..v + e].contains('\n')).map(|e| v + e)
        } else {
            quote.and_then(|q| text[v..].find([q, '\n']).filter(|e| text[v + e..].starts_with(q)).map(|e| v + e))
        };
        let end = quoted_end.unwrap_or_else(|| text[v..].find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | '}')).map_or(text.len(), |e| v + e));
        let value = &text[v..end];
        let counter = lower.contains("token") && ["tokens", "count", "limit", "used", "total", "max", "budget", "len", "length"].iter().any(|w| lower.ends_with(w));
        // A short value with a digit (`hunter2`, `123456`) is still a password; a short word
        // without one (`String`, `string`) is a type name.
        let long_enough = value.len() >= MIN_SECRET_VALUE || value.len() >= SHORT_SECRET_VALUE && value.chars().any(|c| c.is_ascii_digit());
        if long_enough && looks_like_a_literal(value) && !(counter && value.chars().all(|c| c.is_ascii_digit())) {
            out.push(Finding { kind: SecretKind::NamedSecret, range: v..end });
        }
    }
    out
}

/// The identifier that ends just before an `=` or `:` (quotes and spaces skipped).
fn name_before(head: &str) -> &str {
    let head = head.trim_end_matches([' ', '\t', '"', '\'', '\\']);
    let start = head.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')).map_or(0, |p| p + 1);
    &head[start..]
}

/// A value that is the secret itself, not a reference to where it lives.
fn looks_like_a_literal(value: &str) -> bool {
    let reference = value.starts_with(['$', '<', '{', '%']) || value.starts_with("[REDACTED") || value.contains("process.env") || value.contains("env(");
    let placeholder = value.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '.' | '-' | '_'));
    !reference && !placeholder && !value.starts_with("//")
}

#[cfg(test)]
mod tests {
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
    fn url_passwords_are_masked_without_changing_the_url() {
        assert_eq!(redact("visit https://alice:hunter2@example.com/path and postgres://bob:p@db:5432/x").text,
            "visit https://alice:[REDACTED:named-secret]@example.com/path and postgres://bob:[REDACTED:named-secret]@db:5432/x");
        assert_eq!(redact("https://api-key:password@example.com").text, "https://api-key:[REDACTED:named-secret]@example.com");
        assert!(redact("visit https://example.com/path").is_clean());
    }

    #[test]
    fn basic_authorization_is_masked() {
        assert_eq!(redact("Authorization: Basic dXNlcjpwYXNz").text, "Authorization: Basic [REDACTED:bearer-token]");
        assert_eq!(redact("Authorization: Bearer abcdef123456").text, "Authorization: Bearer [REDACTED:bearer-token]");
        assert!(redact("Basic principles are useful").is_clean());
    }

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
    fn credentials_name_is_masked() {
        assert_eq!(redact("credentials: hunter2hunter2").text, "credentials: [REDACTED:named-secret]");
    }

    #[test]
    fn escaped_json_password_is_masked() {
        assert_eq!(redact(r#"{\"password\": \"hunter2hunter2\"}"#).text, r#"{\"password\": \"[REDACTED:named-secret]\"}"#);
    }

    #[test]
    fn google_key_before_non_key_character_is_masked_without_suffix() {
        let key = "AIzaSyA-abcdefghijklmnopqrstuvwxyz01234";
        assert_eq!(key.len(), 39);
        assert_eq!(redact(&format!("{key}/path")).text, "[REDACTED:google-key]/path");
        assert!(redact(&format!("{key}A")).is_clean());
    }

    fn kinds(text: &str) -> Vec<SecretKind> {
        scan(text).into_iter().map(|f| f.kind).collect()
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
    fn a_quoted_multi_word_value_is_blanked_whole() {
        let r = redact(r#"{"password": "my long pass phrase", "user": "bob"}"#);
        assert!(!r.text.contains("long") && !r.text.contains("phrase"), "{}", r.text);
        assert!(r.text.contains("bob"), "{}", r.text);
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
}
