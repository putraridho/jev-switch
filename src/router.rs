//! The router: Jev judges the *task* (not the model name), code maps it to one of
//! three tiers, and confidence gates the cheap route:
//!   simple (confident) → Haiku · reasoning (confident) → Opus · everything else → Sonnet.
//! Both cutoffs are calibrated, not guessed (see README).

use anyhow::Result;
use serde_json::json;

use crate::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use crate::typesafe::{ChoiceAnswer, TypeSafe};

#[derive(Debug)]
pub struct Decision {
    pub model: &'static str,
    pub kind: String,
    pub confidence: f64,
    pub reason: &'static str,
}

/// A code-mode routing judgment for one turn (see [`Router::code_want`]).
pub struct CodeWant {
    pub tier: usize,
    pub confident: bool,
    pub reason: String,
}

/// `needs_changes` probability at or above which a code turn counts as editing.
pub const CODE_EDIT_P: f64 = 0.5;
/// Above this many context tokens the cheap model is skipped: Haiku's window is
/// 200K, and a big context is exactly where a cache rebuild costs the most.
pub const CODE_CHEAP_MAX_CONTEXT: u64 = 120_000;

pub struct Router {
    pub ts: TypeSafe,
    /// Route to the cheap model only when the task is `simple` AND confidence is
    /// at least this. Do NOT guess this — read it off the `tune` sweep.
    pub cheap_threshold: f64,
    /// Route `reasoning` to the strong model only at or above this confidence;
    /// below it, the mid model. 0.0 sends all `reasoning` to strong.
    pub strong_threshold: f64,
}

impl Router {
    pub fn new(ts: TypeSafe, cheap_threshold: f64) -> Self {
        Self { ts, cheap_threshold, strong_threshold: 0.0 }
    }

    pub fn with_strong_threshold(mut self, strong_threshold: f64) -> Self {
        self.strong_threshold = strong_threshold;
        self
    }

    /// The single routing judgment, shared by `route` (production) and `tune`
    /// (calibration) so both evaluate the exact same question.
    pub async fn classify(&self, request: &str, recent_context: &str) -> Result<ChoiceAnswer> {
        self.ts
            .choice(
                json!({ "request": request, "recent_context": recent_context }),
                "What does this coding request need from the model?",
                json!({
                    "simple": {
                        "what": "a direct lookup, formatting, a short edit, a factual answer, or a small self-contained snippet: a one-liner, regex, command, config file, or a short standard function (under ~20 lines) with no multi-step reasoning",
                        "not_for": "anything needing planning, debugging across files, or tradeoff analysis; or any security-sensitive code (auth, crypto, signatures, passwords, tokens, input sanitization, injection)",
                        "examples": ["rename a variable", "what does this flag do", "format this JSON", "write a one-line regex", "check if a string is a palindrome", "write a Dockerfile for a Node app", "find the largest file in a directory"]
                    },
                    "standard": {
                        "what": "everyday coding that needs more than a short snippet but stays within one file or component: a module, class, script, middleware, CLI, tests, a small refactor, or debugging a specific error in given code",
                        "not_for": "one-liners, lookups, or short standard functions (simple), or work spanning many files or systems, architecture, or tradeoffs (reasoning)",
                        "examples": ["write a CLI that parses a CSV and prints a column average", "add pagination to this endpoint", "why does this async function return undefined", "write unit tests for this function"]
                    },
                    "reasoning": {
                        "what": "multi-step problem solving, system design, planning, debugging across files or systems, math or logic, or tradeoff analysis",
                        "examples": ["why does the upload hang on large files", "design the retry and backoff strategy", "refactor this module to remove the cycle"]
                    },
                    "unclear": {
                        "what": "ambiguous, mixed, or under-specified; not clearly simple, standard, or reasoning"
                    }
                }),
            )
            .await
    }

    /// A-priori routing from the request text (+ recent context), for a request
    /// with no prior turn to follow.
    pub async fn route(&self, request: &str, recent_context: &str) -> Result<Decision> {
        self.route_turn(request, recent_context, None).await
    }

    /// Routing for a turn in a conversation. `prev_model` is the model that
    /// answered the previous turn, if any. Precedence:
    /// 1. security-sensitive requests always go strong (hard rule, not Jev's call);
    /// 2. a short follow-up stays on the previous model unless Jev is confident
    ///    about a tier (confidently simple or confidently reasoning);
    /// 3. otherwise Jev's judgment: confidently simple → cheap; confidently
    ///    reasoning → strong; everything else (unsure simple/reasoning, standard,
    ///    unclear) → mid.
    pub async fn route_turn(
        &self,
        request: &str,
        recent_context: &str,
        prev_model: Option<&'static str>,
    ) -> Result<Decision> {
        let ans = self.classify(request, recent_context).await?;
        let go_cheap = ans.choice == "simple" && ans.confidence >= self.cheap_threshold;
        let go_strong = ans.choice == "reasoning" && ans.confidence >= self.strong_threshold;
        let (model, reason) = if is_security_sensitive(request) {
            (MODEL_STRONG, "security-sensitive \u{2192} strong")
        } else if let Some(prev) = prev_model.filter(|_| {
            !go_cheap && !go_strong && is_short_follow_up(request)
        }) {
            (prev, "short follow-up \u{2192} same model as last turn")
        } else if go_cheap {
            (MODEL_CHEAP, "confidently simple")
        } else if go_strong {
            (MODEL_STRONG, "confidently reasoning \u{2192} strong")
        } else {
            (MODEL_MID, "standard / unclear / low-confidence \u{2192} mid")
        };
        Ok(Decision { model, kind: ans.choice, confidence: ans.confidence, reason })
    }

    /// Code mode: probability that fulfilling `request` means modifying files or
    /// running commands (vs. only reading code and answering). Read-only turns are
    /// the only ones the cheap model may take, and it gets read-only tools.
    pub async fn needs_changes(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.ts
            .noul(
                json!({ "request": request, "recent_context": recent_context }),
                "Does fulfilling this request require modifying files or running commands in the project?",
                json!({
                    "true": "editing, creating, or deleting files; running builds, tests, scripts, git, or other commands",
                    "false": "only reading or searching code and answering: explaining, locating, reviewing without changes"
                }),
            )
            .await
    }

    /// Code-mode tier for one turn, before session stickiness. Tiers: 0 cheap,
    /// 1 mid, 2 strong. Strong security signals, confident reasoning, and edits to
    /// auth code → strong; reading auth code → mid; confidently
    /// simple, read-only, small-context → cheap; edits/commands → mid minimum.
    /// If Jev is unavailable, returns `fallback` (unconfident) instead of failing.
    pub async fn code_want(
        &self,
        request: &str,
        recent_context: &str,
        context_tokens: u64,
        fallback: usize,
    ) -> CodeWant {
        let security = security_level(request);
        if security == Security::Strong {
            return CodeWant { tier: 2, confident: true, reason: "security-sensitive".into() };
        }
        let (ans, edit_p) = match tokio::join!(
            self.classify(request, recent_context),
            self.needs_changes(request, recent_context)
        ) {
            (Ok(a), Ok(p)) => (a, p),
            (Err(e), _) | (_, Err(e)) => {
                return CodeWant { tier: fallback, confident: false, reason: format!("jev unavailable ({e:#})") };
            }
        };
        let tag = format!("{} {:.2}, edit p {:.2}", ans.choice, ans.confidence, edit_p);
        if ans.choice == "reasoning" && ans.confidence >= self.strong_threshold {
            return CodeWant { tier: 2, confident: true, reason: format!("confidently reasoning ({tag})") };
        }
        if security == Security::Weak {
            // Auth plumbing: changing it gets strong; reading about it gets mid (never cheap).
            return if edit_p >= CODE_EDIT_P {
                CodeWant { tier: 2, confident: true, reason: format!("changes auth/session code ({tag})") }
            } else {
                CodeWant { tier: 1, confident: true, reason: format!("reads auth/session code ({tag})") }
            };
        }
        if ans.choice == "simple" && ans.confidence >= self.cheap_threshold {
            let (tier, why) = if edit_p >= CODE_EDIT_P {
                (1, "simple, needs changes/commands")
            } else if context_tokens > CODE_CHEAP_MAX_CONTEXT {
                (1, "simple, context too big for cheap")
            } else {
                (0, "simple, read-only")
            };
            return CodeWant { tier, confident: true, reason: format!("{why} ({tag})") };
        }
        CodeWant {
            tier: 1,
            confident: ans.choice == "standard" && ans.confidence >= 0.5,
            reason: format!("everyday ({tag})"),
        }
    }

    /// Verify variant: judge whether a cheap answer actually resolved the request.
    /// Returns true when adequate. Gate its probability on a threshold you also
    /// calibrate; this tracks outcome quality far more stably than a-priori
    /// difficulty, so it is the recommended escalation signal.
    pub async fn answer_is_adequate(
        &self,
        request: &str,
        answer: &str,
        threshold: f64,
    ) -> Result<(bool, f64)> {
        let p = self
            .ts
            .noul(
                json!({ "request": request, "answer": answer }),
                "Does the answer fully and correctly address the request?",
                json!({
                    "true": "complete, correct, and directly responsive",
                    "false": "incomplete, wrong, evasive, or only partially addresses it"
                }),
            )
            .await?;
        Ok((p >= threshold, p))
    }
}

/// Strong signals: vulnerability classes, secrets, crypto, auditing. Matched as
/// whole words (lowercased, split on non-alphanumerics) so "author" or "HashMap"
/// don't hit.
const SECURITY_STRONG_WORDS: &[&str] = &[
    "idor", "injection", "xss", "csrf", "ssrf", "rce", "traversal", "pollution",
    "privilege", "privileges", "exploit", "exploits", "unsafe", "arbitrary",
    "secret", "secrets", "password", "passwords", "passwd", "passphrase",
    "credential", "credentials", "apikey", "hmac", "signature", "signatures",
    "bcrypt", "argon2", "scrypt", "pbkdf2", "cipher", "csp", "escape", "escaping",
    "secure", "securely", "security",
];
const SECURITY_STRONG_PREFIXES: &[&str] = &[
    "vulnerab", "sanitiz", "sanitis", "escalat", "audit", "crypt", "encrypt", "decrypt",
];
const SECURITY_STRONG_PHRASES: &[&str] =
    &["api key", "access key", "private key", "mass assignment", "path travers"];

/// Weak signals: auth plumbing. Routine to read about, risky to change.
const SECURITY_WEAK_WORDS: &[&str] = &[
    "auth", "authn", "authz", "oauth", "oauth2", "oidc", "saml", "sso", "jwt", "jwts",
    "login", "logout", "signin", "signup", "session", "sessions", "cookie", "cookies",
    "token", "tokens", "cors", "permission", "permissions", "rbac", "tls", "ssl",
    "certificate", "certificates", "webhook", "webhooks", "admin", "parameterize",
    "parameterized",
];
const SECURITY_WEAK_PREFIXES: &[&str] = &["authenticat", "authoriz"];
/// Common non-security uses of weak words, removed before matching.
const SECURITY_WEAK_EXCLUDES: &[&str] = &["design token", "color token", "css token", "theme token"];

/// How security-sensitive a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Security {
    None,
    /// Touches auth/session/token plumbing: fine to explain, risky to change.
    Weak,
    /// Vulnerabilities, secrets, crypto, audits: always the strong model.
    Strong,
}

pub fn security_level(request: &str) -> Security {
    let lower = request.to_lowercase();
    if SECURITY_STRONG_PHRASES.iter().any(|p| lower.contains(p)) {
        return Security::Strong;
    }
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let hit = |w: &str, list: &[&str], prefixes: &[&str]| {
        list.contains(&w) || prefixes.iter().any(|p| w.starts_with(p))
    };
    if words(&lower).iter().any(|w| hit(w, SECURITY_STRONG_WORDS, SECURITY_STRONG_PREFIXES)) {
        return Security::Strong;
    }
    let mut weak_text = lower.clone();
    for ex in SECURITY_WEAK_EXCLUDES {
        weak_text = weak_text.replace(ex, " ");
    }
    if words(&weak_text).iter().any(|w| hit(w, SECURITY_WEAK_WORDS, SECURITY_WEAK_PREFIXES)) {
        return Security::Weak;
    }
    Security::None
}

/// Chat mode's hard rule: any security signal goes to the strong model, because
/// a confident-looking wrong answer is costly here and Jev's difficulty judgment
/// alone doesn't reliably catch it.
pub fn is_security_sensitive(request: &str) -> bool {
    security_level(request) != Security::None
}

/// A short, context-dependent follow-up ("go on", "why?", "shorter please").
pub fn is_short_follow_up(request: &str) -> bool {
    request.split_whitespace().count() <= 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_detection() {
        for r in [
            "hash passwords with bcrypt",
            "write a webhook handler that verifies an HMAC signature",
            "sanitize HTML input",
            "escape SQL strings",
            "generate secure random tokens",
            "store the API key in config",
            "encrypt this field",
            "check routes/projects.ts for path traversal",
            "Check pages.ts for IDOR",
            "Audit cards.ts routes for mass assignment",
            "can a member escalate their own role",
        ] {
            assert_eq!(security_level(r), Security::Strong, "{r}");
        }
        for r in [
            "verify a JWT token",
            "set CORS headers",
            "implement session management",
            "where is the login page",
            "where are the Better Auth config and session helpers",
        ] {
            assert_eq!(security_level(r), Security::Weak, "{r}");
        }
        for r in [
            "write a Python dataclass for a book with title, author, and year",
            "use a HashMap to count words",
            "rename the variable usr to user",
            "what does HTTP status 404 mean",
            "where are the --ed-* design tokens defined",
            "thanks!",
        ] {
            assert_eq!(security_level(r), Security::None, "{r}");
        }
    }
}
