//! Secret redaction for anything jev-code writes outside the session: exports, the audit log
//! and shared transcripts. Pure text in, text out, so every rule is a unit test.
//!
//! Three kinds of secret are recognised, none of them by guessing at entropy:
//! well-known token shapes (`sk-...`, `ghp_...`, `AKIA...`, JWTs), the value of a setting
//! whose name says it is secret (`API_KEY=...`, `"password": "..."`), and `Bearer` tokens.
//! PEM private-key blocks are dropped whole.

/// What a secret is replaced by.
pub const MASK: &str = "[REDACTED]";

/// Words in a setting's name that make its value a secret.
const SECRET_NAMES: [&str; 8] = ["secret", "token", "password", "passwd", "apikey", "api_key", "api-key", "private_key"];

/// Token prefixes and the least number of characters after the prefix for a match.
const TOKEN_PREFIXES: [(&str, usize); 10] = [
    ("sk-", 16),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghs_", 20),
    ("ghu_", 20),
    ("github_pat_", 20),
    ("xoxb-", 16),
    ("xoxp-", 16),
    ("AKIA", 16),
    ("AIza", 30),
];

/// The shortest value that is masked after a secret-looking name: shorter ones are flags
/// and ports (`token=1`), not credentials.
const MIN_VALUE: usize = 6;

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/')
}

/// Whether `word` is a token of a known shape.
fn is_token(word: &str) -> bool {
    if TOKEN_PREFIXES.iter().any(|(prefix, rest)| word.strip_prefix(prefix).is_some_and(|r| r.len() >= *rest && r.chars().all(is_word))) {
        return true;
    }
    // A JWT: three dot-separated base64url parts, the first starting `eyJ` (a JSON object).
    let parts: Vec<&str> = word.split('.').collect();
    parts.len() == 3 && word.starts_with("eyJ") && parts.iter().all(|p| p.len() >= 8 && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '=')))
}

/// `line` with token-shaped words masked.
fn mask_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if is_token(word) {
            out.push_str(MASK);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in line.chars() {
        if is_word(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Whether the name before an `=` or `:` says its value is secret.
fn secret_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|w| lower.contains(w)) || lower.ends_with("_key") || lower == "key" || lower.ends_with("-key")
}

/// A name that counts tokens (`max_tokens`, `token_count`, `tokens_used`): a number after
/// it is a size, not a credential.
fn counter_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("token") && ["tokens", "count", "limit", "used", "total", "max", "budget", "len", "length"].iter().any(|w| lower.ends_with(w))
}

/// `line` with the value after a secret-looking name masked: `NAME=value`, `NAME: value` and
/// `"name": "value"` are all one shape (name, separator, value).
fn mask_assignments(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '-')) {
            i += 1;
        }
        if i == start {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let name: String = chars[start..i].iter().collect();
        out.push_str(&name);
        if !secret_name(&name) {
            continue;
        }
        // The separator: an optional closing quote, `=` or `:`, optional spaces, an optional opening quote.
        let mut j = i;
        if chars.get(j).is_some_and(|c| matches!(c, '"' | '\'')) {
            j += 1;
        }
        if !chars.get(j).is_some_and(|c| matches!(c, '=' | ':')) {
            continue;
        }
        j += 1;
        while chars.get(j) == Some(&' ') {
            j += 1;
        }
        if chars.get(j).is_some_and(|c| matches!(c, '"' | '\'')) {
            j += 1;
        }
        let value_start = j;
        while j < chars.len() && !chars[j].is_whitespace() && !matches!(chars[j], '"' | '\'' | ',' | ';' | '}' | ')') {
            j += 1;
        }
        let value: String = chars[value_start..j].iter().collect();
        if value.chars().count() < MIN_VALUE || value.contains(MASK) || (counter_name(&name) && value.chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        out.extend(&chars[i..value_start]);
        out.push_str(MASK);
        i = j;
    }
    out
}

/// `line` with the token after `Bearer` masked.
fn mask_bearer(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let Some(at) = lower.find("bearer ") else { return line.to_string() };
    let from = at + "bearer ".len();
    let rest = &line[from..];
    let len = rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
    if len < MIN_VALUE || rest[..len].contains("REDACTED") {
        return line.to_string();
    }
    format!("{}{MASK}{}", &line[..from], mask_bearer(&rest[len..]))
}

/// `text` with every secret masked. Idempotent: redacting redacted text changes nothing.
pub fn redact(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_key = false;
    for line in text.lines() {
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            in_key = !line.contains("-----END");
            out.push(format!("{MASK} (private key)"));
        } else if in_key {
            in_key = !line.contains("-----END");
        } else {
            out.push(mask_bearer(&mask_assignments(&mask_tokens(line))));
        }
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
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
