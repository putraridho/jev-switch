use super::named::{looks_like_a_literal, name_before, MIN_SECRET_VALUE};
use super::{Finding, SecretKind};

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '/' | '=')
}

/// Prefix-shaped tokens, and the token after `Bearer` or `Basic`.
pub(super) fn tokens(text: &str) -> Vec<Finding> {
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

fn is_jwt(t: &str) -> bool {
    t.starts_with("eyJ") && t.len() >= 30 && {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() == 3 && parts.iter().all(|p| !p.is_empty())
    }
}
