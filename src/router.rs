//! The router: Jev judges the *task* (not the model name), code maps it to one of
//! three tiers, and confidence gates the cheap route:
//!   simple (confident) → Haiku · reasoning (confident) → Opus · everything else → Sonnet.
//! Both cutoffs are calibrated, not guessed (see README).

use anyhow::Result;
use serde_json::{json, Value};

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
    /// Jev's raw signals for the usage log (`Value::Null` when Jev wasn't asked).
    pub signals: Value,
}

/// `needs_changes` probability at or above which a code turn counts as editing.
pub const CODE_EDIT_P: f64 = 0.5;
/// Code mode: `security_review` probability at or above which a turn goes strong
/// even with no security keyword ("can a viewer see X?", "who can call this?").
/// On lunagraph (2,296 prompts, 408 Sonnet answers graded against the code), the
/// keyword-less security questions Sonnet missed and Opus fixed scored 0.86-0.97;
/// 0.85 catches all but one at 0.60, for 0.9% more turns on Opus than 0.90.
pub const CODE_SECURITY_P: f64 = 0.85;
/// Code mode default for `CODE_STRONG_THRESHOLD`: Jev `reasoning` confidence at or
/// above which a turn goes strong. On lunagraph, Sonnet's answers with reasoning
/// confidence 0.75-0.85 were 31 correct and 2 partial that Opus didn't fix either;
/// its misses Opus fixed scored 0.89 and 0.92. Raising it from 0.75 cut Opus turns
/// from 32% to 28% with no fixable miss lost.
pub const CODE_STRONG_THRESHOLD: f64 = 0.85;

/// Most consecutive easier turns (confident or not) a tier is held for. A cache
/// rebuild costs about one turn's worth of re-reading the context at the new model's
/// write price; against a ~5x Opus premium that is repaid in ~3 turns, so holding
/// Opus through 4+ vague turns costs more than it saves. On the logged turns, 5 of
/// 24 holds ran 4-9 turns, all of them unclear/low-confidence ones that never counted
/// toward `downgrade_after`.
pub const MAX_HOLD: u32 = 4;

/// Session stickiness for code-mode tiers. Prompt caches are per model, so a
/// switch rebuilds the whole context cache: upgrade at once, but step down (one
/// tier, never below mid) only after `downgrade_after` confident easier turns.
#[derive(Debug, Default, Clone)]
pub struct Sticky {
    /// Tier of the model that answered the previous turn.
    pub tier: Option<usize>,
    lower_streak: u32,
    /// Consecutive turns wanting a lower tier than `tier`, confident or not.
    held: u32,
}

impl Sticky {
    /// The tier for this turn given Jev's `want`. Callers set `tier` once the
    /// turn runs on it.
    pub fn decide(&mut self, w: &CodeWant, downgrade_after: u32) -> (usize, String) {
        let Some(cur) = self.tier else {
            return (w.tier, w.reason.clone());
        };
        if w.tier > cur {
            self.lower_streak = 0;
            self.held = 0;
            return (w.tier, format!("upgrade: {}", w.reason));
        }
        if w.tier == cur {
            self.lower_streak = 0;
            self.held = 0;
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.held += 1;
        if self.held >= MAX_HOLD && cur > 1 {
            self.lower_streak = 0;
            self.held = 0;
            return (cur - 1, format!("downgrade after holding {MAX_HOLD} easier turns: {}", w.reason));
        }
        if !w.confident {
            // Uncertain turn: keep the streak so one vague message doesn't erase progress.
            return (cur, format!("stay (warm cache): {}", w.reason));
        }
        self.lower_streak += 1;
        if self.lower_streak >= downgrade_after {
            self.lower_streak = 0;
            self.held = 0;
            return ((cur - 1).max(1), format!("downgrade after {downgrade_after} easier turns: {}", w.reason));
        }
        (cur, format!("stay ({}/{} toward downgrade): {}", self.lower_streak, downgrade_after, w.reason))
    }
}

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
    /// running commands (vs. only reading code and answering). Changing auth or
    /// session code goes to the strong model; reading it stays on mid.
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

    /// Code mode: probability that `request` is about security: who can access or
    /// change what, or whether something is safe against misuse. Catches authz and
    /// trust-boundary questions that carry no security keyword, and lets keyword
    /// matches that are about something else (design tokens, React keys) through.
    pub async fn security_review(&self, request: &str, recent_context: &str) -> Result<f64> {
        self.ts
            .noul(
                json!({ "request": request, "recent_context": recent_context }),
                "Is this request about security: who can access or change what, or whether something is safe against misuse or attack?",
                json!({
                    "true": "authorization and access control (can a user, role, or member see or change something they shouldn't), data exposure, trust boundaries, untrusted input (injection, path traversal, SSRF, XSS, unsafe URLs or IPC), secrets and credentials, crypto, signatures, or reviewing code for vulnerabilities",
                    "false": "ordinary coding, debugging, or design; or security-sounding words that mean something else here: design or CSS tokens, React keys, editor or canvas session state, hashing for ids, caching or dedupe, sandboxed iframe layout"
                }),
            )
            .await
    }

    /// Outcome signal: probability that `request` reacts to the previous answer as
    /// wrong, incomplete or not what was wanted (a correction, complaint, retry or
    /// the same question again). Logged for later analysis; never used for routing.
    pub async fn is_correction(&self, request: &str, previous_request: &str, previous_answer: &str) -> Result<f64> {
        self.ts
            .noul(
                json!({ "request": request, "previous_request": previous_request, "previous_answer": previous_answer }),
                "Is this new message a reaction to the previous answer being wrong, incomplete, or not what the user wanted?",
                json!({
                    "true": "a correction, complaint, retry, or the same question asked again; the user is unhappy with or re-asking about what the assistant just did",
                    "false": "a new task, a follow-up building on a good answer, a clarification the user was asked for, or an acknowledgement"
                }),
            )
            .await
    }

    /// Code-mode tier for one turn, before session stickiness: 1 mid or 2 strong.
    /// Code mode never uses the cheap model: across five repos Haiku explanations
    /// held up ~85% at every Jev confidence, so no cutoff could sort out its misses,
    /// and per-model prompt caches make its savings small in a coding session.
    /// Strong security signals, security questions without keywords (Jev
    /// `security_review`), confident reasoning, and edits to auth code → strong;
    /// everything else → mid. If Jev is unavailable, returns `fallback`.
    pub async fn code_want(&self, request: &str, recent_context: &str, fallback: usize) -> CodeWant {
        let security = security_level(request);
        if security == Security::Strong {
            return CodeWant {
                tier: 2,
                confident: true,
                reason: "security-sensitive".into(),
                signals: json!({ "security": "strong" }),
            };
        }
        let (ans, edit_p, sec_p) = match tokio::join!(
            self.classify(request, recent_context),
            self.needs_changes(request, recent_context),
            self.security_review(request, recent_context)
        ) {
            (Ok(a), Ok(p), Ok(s)) => (a, p, s),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                return CodeWant {
                    tier: fallback,
                    confident: false,
                    reason: format!("jev unavailable ({e:#})"),
                    signals: Value::Null,
                };
            }
        };
        let tag = format!("{} {:.2}, edit p {:.2}, sec p {:.2}", ans.choice, ans.confidence, edit_p, sec_p);
        let signals = json!({
            "choice": ans.choice, "confidence": ans.confidence, "edit_p": edit_p, "sec_p": sec_p,
            "security": format!("{security:?}").to_lowercase(),
        });
        if sec_p >= CODE_SECURITY_P {
            return CodeWant { tier: 2, confident: true, reason: format!("security review ({tag})"), signals };
        }
        if ans.choice == "reasoning" && ans.confidence >= self.strong_threshold {
            return CodeWant { tier: 2, confident: true, reason: format!("confidently reasoning ({tag})"), signals };
        }
        if security == Security::Weak && edit_p >= CODE_EDIT_P {
            return CodeWant { tier: 2, confident: true, reason: format!("changes auth/session code ({tag})"), signals };
        }
        // Confident it isn't hard work: lets a session step back down from strong.
        // Short replies ("thanks", "done") come back `unclear` but aren't edits.
        let confident = (matches!(ans.choice.as_str(), "simple" | "standard") && ans.confidence >= 0.5)
            || (ans.choice == "unclear" && edit_p < 0.2);
        CodeWant { tier: 1, confident, reason: format!("everyday ({tag})"), signals }
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
    "secure", "securely", "security", "allowlist", "allowlists", "denylist", "blocklist",
    "isolation", "tenant", "tenants", "leak", "leaks", "leakage", "leaking", "redos", "untrusted",
    "bypass", "forge", "forged", "forgery", "noopener", "tabnabbing", "dangerously",
];
const SECURITY_STRONG_PREFIXES: &[&str] = &[
    "vulnerab", "sanitiz", "sanitis", "escalat", "audit", "crypt", "encrypt", "decrypt",
    "inject", "poison", "exfiltrat",
];
const SECURITY_STRONG_PHRASES: &[&str] =
    &["api key", "access key", "private key", "mass assignment", "path travers", "javascript:"];

/// Weak signals: auth plumbing. Routine to read about, risky to change.
const SECURITY_WEAK_WORDS: &[&str] = &[
    "auth", "authn", "authz", "oauth", "oauth2", "oidc", "saml", "sso", "jwt", "jwts",
    "login", "logout", "signin", "signup", "session", "sessions", "cookie", "cookies",
    "token", "tokens", "cors", "permission", "permissions", "rbac", "tls", "ssl",
    "certificate", "certificates", "webhook", "webhooks", "admin", "parameterize",
    "parameterized", "redirect", "redirects", "origin", "origins", "postmessage", "turnstile",
    "captcha",
];
const SECURITY_WEAK_PREFIXES: &[&str] = &["authenticat", "authoriz", "sandbox"];
/// Common non-security uses of weak words, removed before matching.
const SECURITY_WEAK_EXCLUDES: &[&str] = &["design token", "color token", "css token", "theme token"];

/// "securityLevel" → "security Level", so identifiers match like words. Keeps the
/// original word too ("postMessage" also stays "postmessage").
fn split_camel(text: &str) -> String {
    let mut split = String::with_capacity(text.len() + 8);
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_uppercase() && prev_lower {
            split.push(' ');
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        split.push(c);
    }
    format!("{text} {split}")
}

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
    let lower = split_camel(request).to_lowercase();
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

    fn want(tier: usize, confident: bool) -> CodeWant {
        CodeWant { tier, confident, reason: String::new(), signals: Value::Null }
    }

    #[test]
    fn uncertain_turn_holds_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // 1/2
        assert_eq!(s.decide(&want(1, false), 2).0, 2); // vague turn: streak kept
        assert_eq!(s.decide(&want(1, true), 2).0, 1); // 2/2: step down
    }

    #[test]
    fn hard_turn_resets_downgrade_streak() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        assert_eq!(s.decide(&want(1, true), 2).0, 2);
        assert_eq!(s.decide(&want(2, true), 2).0, 2); // needs Opus: streak reset
        assert_eq!(s.decide(&want(1, true), 2).0, 2); // back to 1/2
    }

    #[test]
    fn vague_turns_cannot_hold_a_tier_forever() {
        let mut s = Sticky { tier: Some(2), ..Default::default() };
        for _ in 1..MAX_HOLD {
            assert_eq!(s.decide(&want(1, false), 2).0, 2);
        }
        assert_eq!(s.decide(&want(1, false), 2).0, 1);
    }

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
            "call_tool.py lets a workflow call tools by name. Is there an allowlist?",
            "Review storage.py for cross-tenant file leakage",
            "Is there any way to inject a system message through the API?",
            "is the mermaid securityLevel set to strict",
            "do we block javascript: and data: URLs when pasting links",
            "can markdown images exfiltrate data via query strings",
            "never use dangerouslySetInnerHTML here",
        ] {
            assert_eq!(security_level(r), Security::Strong, "{r}");
        }
        for r in [
            "verify a JWT token",
            "set CORS headers",
            "implement session management",
            "where is the login page",
            "where are the Better Auth config and session helpers",
            "Are there CPU and memory limits on sandboxed execution?",
            "How does use_sandbox.py decide which backend to use?",
            "check each window.addEventListener('message') handler validates the origin",
            "validate the redirect path in routes-utils.ts",
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
