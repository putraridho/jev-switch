//! Everything that fences in or watches the router, in one place: the privacy policy and its
//! audit log, the user's routing rules, the budget cap and meter, the failover policy and the
//! provider scoreboard. [`crate::turn::Engine`] holds one [`Guards`] and asks it two things:
//! which of the usable models may take this turn ([`Guards::restrict`]) and what may leave
//! with the prompt ([`Guards::outgoing`]).

use std::path::{Path, PathBuf};

use crate::backend::ProviderId;
use crate::budget::{Budget, Meter, PriceTable};
use crate::catalog::{TaskType, Usable};
use crate::failover::FailoverPolicy;
use crate::privacy::{self, AuditLog, AuditRecord, Outgoing, Policy};
use crate::routing_rules::{self, Rules};
use crate::scoreboard::Scoreboard;

/// What `restrict` needs to know about the turn.
pub struct TurnFacts<'a> {
    pub root: &'a Path,
    pub prompt: &'a str,
    /// Jev's task type, when it judged the turn.
    pub task: Option<TaskType>,
    /// Attached images, which leave the machine with the prompt.
    pub attached: &'a [PathBuf],
    /// The person pinned a model with `/model`: user routing rules step aside.
    pub pinned: bool,
    /// Models already tried this turn (a failover is under way when above 0).
    pub attempts: u32,
}

/// The models that may take the turn, and what was done to get there.
#[derive(Debug, Clone)]
pub struct Restricted {
    pub usable: Usable,
    /// Rules that fired, for the route line.
    pub notes: Vec<String>,
    /// Providers the privacy policy kept the prompt from, with why.
    pub blocked: Vec<(ProviderId, String)>,
}

#[derive(Debug, Default)]
pub struct Guards {
    pub privacy: Policy,
    pub audit: Option<AuditLog>,
    pub rules: Rules,
    pub budget: Budget,
    pub prices: PriceTable,
    pub failover: FailoverPolicy,
    pub meter: Meter,
    pub scoreboard: Scoreboard,
    /// Lines of the settings files that could not be read, shown once by `/privacy`.
    pub problems: Vec<String>,
}

impl Guards {
    /// Every setting file of the user's state folder and the project. A missing file is
    /// the default; a broken line is kept in `problems` and skipped. `JEV_LOCAL_ONLY=1`
    /// turns local-only mode on whatever the files say.
    pub fn load(state_dir: Option<&Path>, project: &Path) -> Self {
        let (privacy, privacy_errors) = Policy::load(state_dir, project);
        let (rules, rule_errors) = Rules::load(state_dir, project);
        let local_only = crate::envfile::lookup("JEV_LOCAL_ONLY").is_some_and(|v| matches!(v.trim(), "1" | "on" | "true" | "yes"));
        let mut problems: Vec<String> = privacy_errors.iter().map(|(file, e)| format!("{}: {e}", file.display())).collect();
        problems.extend(rule_errors.iter().map(|e| format!("routing.rules line {}: {}", e.line, e.message)));
        Self {
            privacy: privacy.with_local_only(local_only),
            audit: state_dir.map(AuditLog::in_state_dir),
            rules,
            budget: Budget::load(state_dir),
            prices: PriceTable::load(state_dir),
            failover: FailoverPolicy::load(state_dir),
            problems,
            ..Self::default()
        }
    }

    /// Narrow `usable` for this turn: privacy policy, budget cap, failover policy, then the
    /// user's routing rules. `Err` is a plain sentence when a step that started with models
    /// left none.
    pub fn restrict(&self, usable: &Usable, turn: &TurnFacts) -> Result<Restricted, String> {
        let verdict = self.privacy.check(turn.root, turn.prompt, turn.attached);
        let blocked: Vec<(ProviderId, String)> =
            crate::providers::ALL_PROVIDERS.into_iter().filter(|p| !usable.filter(|q, _| q == *p).is_empty()).filter_map(|p| verdict.denial(p).map(|why| (p, why))).collect();
        let mut current = usable.filter(|p, _| verdict.permits(p));
        if current.is_empty() && !usable.is_empty() {
            return Err(verdict.refusal_line());
        }
        let may_pay = self.budget.allows_paid(&self.meter) && (self.failover.paid_fallback || turn.attempts == 0);
        if !may_pay {
            let free = current.filter(|p, m| !crate::budget::is_paid(p, m));
            if free.is_empty() && !current.is_empty() {
                let why = if self.budget.allows_paid(&self.meter) { "paid-fallback is off (/failover)" } else { "the budget cap is reached (/budget)" };
                return Err(format!("only models that cost money are left, and {why}"));
            }
            current = free;
        }
        let mut notes = Vec::new();
        if !turn.pinned && !self.rules.is_empty() {
            let facts = routing_rules::Facts::new(turn.root, turn.prompt, turn.task);
            let applied = self.rules.apply(&current, &facts)?;
            current = applied.usable;
            notes = applied.notes;
        }
        Ok(Restricted { usable: current, notes, blocked })
    }

    /// The prompt as it may leave for `provider`, after the secret scan; the audit log
    /// gets a line saying what was sent.
    pub fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> Outgoing {
        let out = privacy::prepare(&self.privacy, prompt);
        if let Some(log) = &self.audit {
            log.record(&AuditRecord::sent(now_ms, provider, model, &out));
        }
        out
    }

    /// Note in the audit log that the policy kept the prompt from providers.
    pub fn audit_blocked(&self, blocked: &[(ProviderId, String)], now_ms: u64) {
        if let Some(log) = &self.audit {
            for (provider, why) in blocked {
                log.record(&AuditRecord::blocked(now_ms, *provider, why.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::Cap;
    use crate::privacy::Source;

    fn facts<'a>(prompt: &'a str, attempts: u32) -> TurnFacts<'a> {
        TurnFacts { root: Path::new("/w"), prompt, task: None, attached: &[], pinned: false, attempts }
    }

    fn all() -> Usable {
        Usable::all_of(crate::providers::ALL_PROVIDERS)
    }

    fn has(u: &Usable, p: ProviderId) -> bool {
        !u.filter(|q, _| q == p).is_empty()
    }

    fn guards(privacy: &str) -> Guards {
        Guards { privacy: Policy::parse(privacy, Source::User).0, ..Guards::default() }
    }

    #[test]
    fn the_default_changes_nothing() {
        let r = Guards::default().restrict(&all(), &facts("anything", 0)).unwrap();
        assert!(r.blocked.is_empty() && r.notes.is_empty());
        assert!(crate::providers::ALL_PROVIDERS.iter().all(|p| has(&r.usable, *p)));
    }

    #[test]
    fn privacy_removes_providers_and_says_who() {
        let g = guards("block secrets/** from deepseek");
        let r = g.restrict(&all(), &facts("fix secrets/a.yml", 0)).unwrap();
        assert!(!has(&r.usable, ProviderId::DeepSeek) && has(&r.usable, ProviderId::Claude));
        assert_eq!(r.blocked.len(), 1);
        assert_eq!(r.blocked[0].0, ProviderId::DeepSeek);
        // Everything refused: a sentence, not an empty set.
        let g = guards("local-only on");
        assert!(g.restrict(&all(), &facts("x", 0)).unwrap_err().contains("local-only"));
        // Nothing usable to begin with is the normal "nothing usable" path, not a refusal.
        assert!(g.restrict(&Usable::default(), &facts("x", 0)).unwrap().usable.is_empty());
    }

    #[test]
    fn the_budget_cap_removes_paid_models_only_once_reached() {
        let mut g = Guards { budget: Budget { cap: Some(Cap::Tokens(100)) }, ..Guards::default() };
        let usage = crate::backend::Usage { input_tokens: 200, ..Default::default() };
        g.meter.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage, &g.prices);
        let r = g.restrict(&all(), &facts("x", 0)).unwrap();
        assert!(!has(&r.usable, ProviderId::DeepSeek) && has(&r.usable, ProviderId::Claude));
        // Only paid models left: refused with the reason.
        let err = g.restrict(&Usable::all_of([ProviderId::DeepSeek]), &facts("x", 0)).unwrap_err();
        assert!(err.contains("budget cap is reached"), "{err}");
    }

    #[test]
    fn paid_fallback_off_blocks_paid_models_only_during_a_failover() {
        let g = Guards { failover: FailoverPolicy { max_attempts: 6, paid_fallback: false }, ..Guards::default() };
        assert!(has(&g.restrict(&all(), &facts("x", 0)).unwrap().usable, ProviderId::DeepSeek));
        let r = g.restrict(&all(), &facts("x", 1)).unwrap();
        assert!(!has(&r.usable, ProviderId::DeepSeek) && has(&r.usable, ProviderId::Codex));
        let err = g.restrict(&Usable::all_of([ProviderId::DeepSeek]), &facts("x", 1)).unwrap_err();
        assert!(err.contains("paid-fallback is off"), "{err}");
    }

    #[test]
    fn user_rules_apply_after_policy_and_a_pin_skips_them() {
        let (rules, _) = Rules::parse("only codex");
        let g = Guards { rules, ..guards("block x/** from codex") };
        // Policy removed codex from this prompt, then `only codex` cannot be met.
        let err = g.restrict(&all(), &facts("see x/a", 0)).unwrap_err();
        assert!(err.contains("only codex"), "{err}");
        let r = g.restrict(&all(), &facts("hello", 0)).unwrap();
        assert!(has(&r.usable, ProviderId::Codex) && !has(&r.usable, ProviderId::Claude));
        assert_eq!(r.notes, vec!["rule: only codex"]);
        let pinned = TurnFacts { pinned: true, ..facts("hello", 0) };
        assert!(has(&g.restrict(&all(), &pinned).unwrap().usable, ProviderId::Claude));
    }

    #[test]
    fn outgoing_blanks_secrets_and_audits_without_the_secret() {
        let d = std::env::temp_dir().join(format!("jev-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = Guards { audit: Some(AuditLog::in_state_dir(&d)), ..Guards::default() };
        let out = g.outgoing("use DB_PASSWORD=hunter2hunter2", ProviderId::Claude, "opus", 7);
        assert_eq!(out.prompt, "use DB_PASSWORD=[REDACTED:named-secret]");
        g.audit_blocked(&[(ProviderId::Grok, "local-only mode is on".into())], 8);
        let raw = std::fs::read_to_string(d.join("privacy-audit.jsonl")).unwrap();
        assert_eq!(raw.lines().count(), 2);
        assert!(!raw.contains("hunter2"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn files_load_and_problems_are_kept() {
        let base = std::env::temp_dir().join(format!("jev-guard-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (state, project) = (base.join("s"), base.join("p"));
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(project.join(".jev")).unwrap();
        std::fs::write(state.join("privacy.rules"), "block a/** from grok\nnonsense\n").unwrap();
        std::fs::write(state.join("routing.rules"), "prefer claude\n").unwrap();
        std::fs::write(state.join("budget"), "usd 3").unwrap();
        std::fs::write(state.join("failover"), "max-attempts 2").unwrap();
        let g = Guards::load(Some(&state), &project);
        assert_eq!(g.privacy.rule_count(), 1);
        assert_eq!(g.problems.len(), 1);
        assert_eq!(g.budget.cap, Some(Cap::Usd(3.0)));
        assert_eq!(g.failover.max_attempts, 2);
        assert!(!g.rules.is_empty() && g.audit.is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
