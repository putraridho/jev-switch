//! The text of `/providers` and the startup summary.

use crate::providers::domain::availability;
use crate::providers::domain::login_hints::{login_target, unsupported_hint, unusable_here_hint};
use crate::shared::domain::duration_text::format_left;
use crate::shared::ALL_PROVIDERS;

use super::{BlockReason, Runtime};

impl Runtime {
    /// The text of `/providers`: probe results, then what has changed since.
    pub fn render(&self, now_ms: u64) -> String {
        let mut out = vec![availability::render(&self.avail)];
        out.extend(self.cooldown_lines(now_ms));
        out.extend(self.unusable_lines());
        out.extend(self.blocked_lines());
        out.push(format!(
            "allowed: credits {}, per-token {}",
            yes_no(self.allow.credits),
            yes_no(self.allow.per_token)
        ));
        out.extend(self.overrides.notes.iter().map(|n| format!("override: {n}")));
        out.join("\n")
    }

    /// The compact startup view: only providers that are ready, any cooldowns carried
    /// over from earlier runs, and where to find the rest.
    pub fn render_summary(&self, now_ms: u64) -> String {
        let mut lines: Vec<String> = Vec::new();
        let ready = availability::render_ready(&self.avail);
        lines.push(if ready.is_empty() { "no provider is ready".to_string() } else { ready });
        lines.extend(self.cooldown_lines(now_ms));
        lines.push("/providers for details".to_string());
        lines.join("\n")
    }

    fn cooldown_lines(&self, now_ms: u64) -> Vec<String> {
        let cooldowns = self.current_cooldowns();
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| Some(format!("{p} cooling for {}", format_left(cooldowns.active(*p, now_ms)?))))
            .collect()
    }

    /// One line per provider blocked for this session, with why and what clears it.
    fn blocked_lines(&self) -> Vec<String> {
        ALL_PROVIDERS
            .iter()
            .filter_map(|p| {
                let line = match self.blocked.get(p)? {
                    BlockReason::Auth => format!(
                        "{p} blocked this session: not logged in (log in to {}, then /providers refresh)",
                        login_target(*p)
                    ),
                    BlockReason::UnusableHere => {
                        format!("{p} blocked this session: {} (/new clears it)", unusable_here_hint(*p))
                    }
                    BlockReason::Unsupported => format!("{p} blocked this session: {}", unsupported_hint(*p)),
                };
                Some(line)
            })
            .collect()
    }

    fn unusable_lines(&self) -> Vec<String> {
        let mut models: Vec<_> = self.unusable_models.iter().collect();
        models.sort();
        models.into_iter().map(|(p, m)| format!("{p}:{m} refused by the provider, not offered")).collect()
    }
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}
