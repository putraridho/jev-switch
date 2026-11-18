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
const SECRET_NAME_WORDS: [&str; 8] = ["secret", "token", "password", "passwd", "api_key", "apikey", "private_key", "access_key"];
/// A value shorter than this is a flag or a placeholder, not a credential.
const MIN_SECRET_VALUE: usize = 8;

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

/// Every secret in `text`, in order, without overlaps.
pub fn scan(text: &str) -> Vec<Finding> {
    let mut found = private_keys(text);
    found.extend(tokens(text));
    found.extend(named_secrets(text));
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

/// Prefix-shaped tokens, and the token after `Bearer`.
fn tokens(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut previous_word: Option<(usize, usize)> = None;
    let flush = |from: usize, to: usize, out: &mut Vec<Finding>, prev: &mut Option<(usize, usize)>| {
        // A sentence's full stop is not part of the token.
        let to = from + text[from..to].trim_end_matches('.').len();
        let token = &text[from..to];
        let after_bearer = prev.is_some_and(|(a, b)| text[a..b].eq_ignore_ascii_case("bearer"));
        if let Some(kind) = token_kind(token) {
            out.push(Finding { kind, range: from..to });
        } else if after_bearer && token.len() >= 16 {
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

fn token_kind(t: &str) -> Option<SecretKind> {
    let len = t.len();
    let tail_is_key = |prefix: &str, min: usize| t.starts_with(prefix) && len >= min;
    if tail_is_key("sk-ant-", 20) {
        Some(SecretKind::AnthropicKey)
    } else if tail_is_key("sk-", 20) {
        Some(SecretKind::OpenAiStyleKey)
    } else if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"].iter().any(|p| tail_is_key(p, 36)) || tail_is_key("github_pat_", 30) {
        Some(SecretKind::GithubToken)
    } else if (t.starts_with("AKIA") || t.starts_with("ASIA"))
        && len == 20
        && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        Some(SecretKind::AwsAccessKey)
    } else if t.starts_with("xox") && t[3..].starts_with(['b', 'a', 'p', 'r', 's']) && t[4..].starts_with('-') && len >= 15 {
        Some(SecretKind::SlackToken)
    } else if t.starts_with("AIza") && len == 39 {
        Some(SecretKind::GoogleKey)
    } else if is_jwt(t) {
        Some(SecretKind::Jwt)
    } else {
        None
    }
}

fn is_jwt(t: &str) -> bool {
    t.starts_with("eyJ") && t.len() >= 30 && {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
    }
}

/// `NAME=value`, `NAME: value`, `"name": "value"` where NAME says it holds a secret.
fn named_secrets(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        if c != '=' && c != ':' {
            continue;
        }
        let name = name_before(&text[..i]);
        let lower = name.to_ascii_lowercase();
        if name.is_empty() || !SECRET_NAME_WORDS.iter().any(|w| lower.contains(w)) {
            continue;
        }
        let mut v = i + 1;
        let mut quote = None;
        while v < bytes.len() && matches!(bytes[v], b' ' | b'\t' | b'"' | b'\'') {
            if bytes[v] == b'"' || bytes[v] == b'\'' {
                quote = Some(bytes[v] as char);
            }
            v += 1;
        }
        // A quoted value runs to its closing quote on the same line, spaces and all.
        let quoted_end = quote.and_then(|q| text[v..].find(|ch: char| ch == q || ch == '\n').filter(|e| text[v + e..].starts_with(q)).map(|e| v + e));
        let end = quoted_end.unwrap_or_else(|| text[v..].find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | ';' | ')' | '}')).map_or(text.len(), |e| v + e));
        let value = &text[v..end];
        if value.len() >= MIN_SECRET_VALUE && looks_like_a_literal(value) {
            out.push(Finding { kind: SecretKind::NamedSecret, range: v..end });
        }
    }
    out
}

/// The identifier that ends just before an `=` or `:` (quotes and spaces skipped).
fn name_before(head: &str) -> &str {
    let head = head.trim_end_matches([' ', '\t', '"', '\'']);
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

    fn kinds(text: &str) -> Vec<SecretKind> {
        scan(text).into_iter().map(|f| f.kind).collect()
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
