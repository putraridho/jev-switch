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

use named::named_secrets;
use private_key::private_keys;
use tokens::tokens;
use url::url_passwords;

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

mod named;
mod private_key;
mod tokens;
mod url;

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

#[cfg(test)]
mod tests;
