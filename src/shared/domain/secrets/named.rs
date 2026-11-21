use super::{Finding, SecretKind};

/// Words in a variable name that say its value is a secret.
const SECRET_NAME_WORDS: [&str; 10] = ["secret", "token", "password", "passwd", "api_key", "api-key", "apikey", "private_key", "access_key", "credentials"];
/// A value shorter than this is a flag, a placeholder or a type name (`password: String`),
/// not a credential.
pub(super) const MIN_SECRET_VALUE: usize = 8;
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

/// `NAME=value`, `NAME: value`, `"name": "value"` where NAME says it holds a secret.
pub(super) fn named_secrets(text: &str, urls: &[Finding]) -> Vec<Finding> {
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
pub(super) fn name_before(head: &str) -> &str {
    let head = head.trim_end_matches([' ', '\t', '"', '\'', '\\']);
    let start = head.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')).map_or(0, |p| p + 1);
    &head[start..]
}

/// A value that is the secret itself, not a reference to where it lives.
pub(super) fn looks_like_a_literal(value: &str) -> bool {
    let reference = value.starts_with(['$', '<', '{', '%']) || value.starts_with("[REDACTED") || value.contains("process.env") || value.contains("env(");
    let placeholder = value.chars().all(|c| matches!(c, '*' | 'x' | 'X' | '.' | '-' | '_'));
    !reference && !placeholder && !value.starts_with("//")
}
