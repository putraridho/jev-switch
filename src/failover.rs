//! How a turn fails over, as the user can set it, and the plain-language cost of doing so.
//!
//! The policy file `<state>/failover` holds lines `max-attempts <n>` (how many models one
//! turn may try, at most [`crate::turn::MAX_ATTEMPTS`]) and `paid-fallback on|off` (whether
//! a failover may land on a model that costs money instead of using a plan). The router
//! itself still decides which model comes next; this only fences it in.

use std::path::Path;

use crate::budget::{short_count, Price};

const FILE: &str = "failover";
/// Below this much context the lost cache is not worth a sentence.
const CACHE_NOTICE_TOKENS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailoverPolicy {
    pub max_attempts: u32,
    pub paid_fallback: bool,
}

impl Default for FailoverPolicy {
    fn default() -> Self {
        Self { max_attempts: crate::turn::MAX_ATTEMPTS, paid_fallback: true }
    }
}

impl FailoverPolicy {
    /// Change one setting from words, like `/failover max-attempts 2` or `/failover paid-fallback off`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "max-attempts" => {
                let n: u32 = value.parse().map_err(|_| "max-attempts needs a number".to_string())?;
                if !(1..=crate::turn::MAX_ATTEMPTS).contains(&n) {
                    return Err(format!("max-attempts is between 1 and {}", crate::turn::MAX_ATTEMPTS));
                }
                self.max_attempts = n;
            }
            "paid-fallback" => {
                self.paid_fallback = match value {
                    "on" => true,
                    "off" => false,
                    _ => return Err("paid-fallback is `on` or `off`".into()),
                }
            }
            other => return Err(format!("unknown setting `{other}` (max-attempts, paid-fallback)")),
        }
        Ok(())
    }

    pub fn parse(text: &str) -> Self {
        let mut policy = Self::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(char::is_whitespace) {
                // A bad line is skipped; the rest still applies.
                let _ = policy.set(k.trim(), v.trim());
            }
        }
        policy
    }

    pub fn load(state_dir: Option<&Path>) -> Self {
        state_dir.and_then(|d| std::fs::read_to_string(d.join(FILE)).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, state_dir: &Path) -> std::io::Result<()> {
        crate::handoff::write_atomic(&state_dir.join(FILE), &self.to_string())
    }
}

impl std::fmt::Display for FailoverPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let paid = if self.paid_fallback { "on" } else { "off" };
        write!(f, "max-attempts {}\npaid-fallback {paid}", self.max_attempts)
    }
}

/// The sentence added to a failover note: the next model has no cache of this conversation,
/// so it reads it all again. `price` is the failed model's, to put a figure on it.
pub fn cache_clause(context_tokens: u64, price: Option<&Price>) -> Option<String> {
    if context_tokens < CACHE_NOTICE_TOKENS {
        return None;
    }
    let tokens = short_count(context_tokens);
    Some(match price.map(|p| p.uncached_extra(context_tokens)).filter(|extra| *extra >= 0.005) {
        Some(extra) => format!("it starts without a cache and reads about {tokens} tokens of this conversation again (about ${extra:.2} more)"),
        None => format!("it starts without a cache and reads about {tokens} tokens of this conversation again"),
    })
}

/// A failover note with the cache sentence added in brackets.
pub fn note_with_cache(note: &str, context_tokens: u64, price: Option<&Price>) -> String {
    match cache_clause(context_tokens, price) {
        Some(clause) => format!("{note} ({clause})"),
        None => note.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_changes_nothing() {
        assert_eq!(FailoverPolicy::default(), FailoverPolicy { max_attempts: crate::turn::MAX_ATTEMPTS, paid_fallback: true });
    }

    #[test]
    fn settings_are_checked() {
        let mut p = FailoverPolicy::default();
        p.set("max-attempts", "2").unwrap();
        p.set("paid-fallback", "off").unwrap();
        assert_eq!(p, FailoverPolicy { max_attempts: 2, paid_fallback: false });
        for (k, v) in [("max-attempts", "0"), ("max-attempts", "99"), ("max-attempts", "x"), ("paid-fallback", "maybe"), ("colour", "red")] {
            assert!(p.set(k, v).is_err(), "{k} {v}");
        }
        assert_eq!(p, FailoverPolicy { max_attempts: 2, paid_fallback: false });
    }

    #[test]
    fn text_round_trips_and_bad_lines_are_skipped() {
        let p = FailoverPolicy { max_attempts: 3, paid_fallback: false };
        assert_eq!(FailoverPolicy::parse(&p.to_string()), p);
        assert_eq!(FailoverPolicy::parse("junk\nmax-attempts 4\nmax-attempts 400"), FailoverPolicy { max_attempts: 4, paid_fallback: true });
    }

    #[test]
    fn the_policy_is_kept() {
        let d = std::env::temp_dir().join(format!("jev-failover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(FailoverPolicy::load(Some(&d)), FailoverPolicy::default());
        let p = FailoverPolicy { max_attempts: 2, paid_fallback: false };
        p.save(&d).unwrap();
        assert_eq!(FailoverPolicy::load(Some(&d)), p);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_cache_sentence() {
        assert_eq!(cache_clause(4_999, None), None);
        assert_eq!(cache_clause(24_000, None).unwrap(), "it starts without a cache and reads about 24k tokens of this conversation again");
        let price = Price { input: 3.0, cached_input: 0.3, output: 15.0 };
        // 200k tokens * (3.0 - 0.3) / 1M = $0.54
        assert!(cache_clause(200_000, Some(&price)).unwrap().ends_with("again (about $0.54 more)"));
        // A saving too small to mention is left out.
        assert!(!cache_clause(6_000, Some(&Price { input: 0.1, cached_input: 0.09, output: 1.0 })).unwrap().contains('$'));
        assert_eq!(note_with_cache("codex hit its limit: trying the next model", 100, None), "codex hit its limit: trying the next model");
        assert!(note_with_cache("x", 12_000, None).starts_with("x (it starts without a cache"));
    }
}
