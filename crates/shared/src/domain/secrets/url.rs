use super::{Finding, SecretKind};

/// Passwords in URL userinfo, without the surrounding URL.
pub(super) fn url_passwords(text: &str) -> Vec<Finding> {
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
