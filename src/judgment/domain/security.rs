//! Keyword-based security detection: the hard rule that sends security-sensitive requests to the strong model.

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
