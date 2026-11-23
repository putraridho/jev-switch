//! Everything that fences in or watches the router, in one place: the privacy policy and its
//! audit log, the user's routing rules, the budget cap and meter, the failover policy and the
//! provider scoreboard. [`crate::application::engine::Engine`] holds one [`Guards`] and asks it two things:
//! which of the usable models may take this turn ([`Guards::restrict`]) and what may leave
//! with the prompt ([`Guards::outgoing`]).

use std::path::{Path, PathBuf};

use jev_backend::ProviderId;
use jev_shared::application::host::Host;
use crate::domain::admission::Estimate;
use crate::domain::budget::{Budget, Meter, PriceTable};
use crate::application::usage_account::{Overview, UsageAccount};
use jev_orchestration::application::ports::BudgetHold;
use jev_backend::Usage;
use jev_catalog::domain::model_table::{TaskType, Usable};
use crate::domain::failover_policy::FailoverPolicy;
use crate::domain::privacy::outgoing::{self, Outgoing};
use crate::domain::privacy::verdict::Verdict;
use crate::domain::privacy::Policy;
use crate::application::privacy_audit::AuditLog;
use crate::application::ports::PrivacyAudit;
use crate::domain::privacy::audit::AuditRecord;
use std::sync::Arc;
use jev_routing::domain::user_rules as routing_rules;
use jev_routing::domain::user_rules::Rules;
use crate::domain::scoreboard::Scoreboard;

/// What `restrict` needs to know about the turn.
pub struct TurnFacts<'a> {
    pub root: &'a Path,
    pub prompt: &'a str,
    /// The catch-up text each provider that is behind would get in front of the prompt: what
    /// the privacy rules cover in it holds that provider back as much as the new prompt does.
    pub catch_ups: &'a [(ProviderId, String)],
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
    pub home: Option<String>,
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
    /// The usage ledger this process shares its spend and reservations through; `None` keeps
    /// the spend in this process only, with nothing reserved.
    pub(crate) shared: Option<UsageAccount>,
}

impl Guards {
    /// Use the detected funding of each provider's CLI to tell paid from plan use.
    pub fn set_fundings(&mut self, fundings: jev_providers::domain::funding::Fundings) {
        self.meter.set_fundings(fundings);
    }

    /// Whether using `model` is paid use under the detected funding.
    pub fn is_paid(&self, provider: ProviderId, model: &str) -> bool {
        self.meter.is_paid(provider, model)
    }

    /// Whether a paid model may still be picked: under the session cap and the daily cap, and
    /// the attempt is the first or paid-fallback is on. A first screen only: [`Guards::hold`]
    /// is the check that reserves.
    pub fn may_pay(&self, attempts: u32) -> bool {
        self.budget.allows_paid(&self.total_meter()) && self.allows_paid_today() && (self.failover.paid_fallback || attempts == 0)
    }

    fn allows_paid_today(&self) -> bool {
        self.budget.daily.is_none() || self.shared.as_ref().is_none_or(|a| self.budget.allows_paid_today(&a.day(&self.prices, self.meter.fundings())))
    }

    /// Share this process's usage through `account`.
    pub fn with_ledger(mut self, account: UsageAccount) -> Self {
        self.shared = Some(account);
        self
    }

    /// Reserve the budget for an attempt of `model` whose prompt is about `prompt_tokens`:
    /// `Err` is why it may not start (see [`crate::domain::admission`]). Every `Ok` hold ends
    /// in [`Guards::settle`] or [`Guards::release`].
    pub fn hold(&self, provider: ProviderId, model: &str, prompt_tokens: u64) -> Result<BudgetHold, String> {
        let estimate = Estimate::of(model, prompt_tokens, &self.prices);
        match &self.shared {
            Some(account) => account.hold(&self.budget, &self.prices, self.meter.fundings(), provider, model, estimate),
            // No ledger: this process's own meter is the whole truth and nothing is reserved.
            None => {
                let (own, none) = (self.meter.clone(), crate::domain::usage_ledger::Reserved::default());
                let standing = crate::domain::admission::Standing { session: &own, session_reserved: &none, day: &own, day_reserved: &none };
                let attempt = crate::domain::admission::Attempt { provider, paid: self.is_paid(provider, model), estimate, budget: &self.budget };
                match crate::domain::admission::admit(&attempt, &standing) {
                    crate::domain::admission::Admission::Admitted { warning } => Ok(BudgetHold { id: None, warning }),
                    crate::domain::admission::Admission::Refused(why) => Err(why),
                }
            }
        }
    }

    /// End a reservation with what the attempt used: in this process's meter and in the ledger.
    pub fn settle(&mut self, hold: &BudgetHold, provider: ProviderId, model: &str, usage: &Usage) {
        self.meter.record(provider, model, usage, &self.prices);
        if let Some(account) = &self.shared {
            account.settle(hold, provider, model, usage);
        }
    }

    /// End the reservation of an attempt that never started.
    pub fn release(&self, hold: &BudgetHold) {
        if let Some(account) = &self.shared {
            account.release(hold);
        }
    }

    /// Record a finished attempt that held nothing.
    pub fn record_spend(&mut self, provider: ProviderId, model: &str, usage: &Usage) {
        self.settle(&BudgetHold::default(), provider, model, usage);
    }

    /// What the whole session has spent: this process's meter plus what the other processes
    /// of the session (the workers, or the planner) put in the shared ledger.
    pub fn total_meter(&self) -> Meter {
        let mut total = self.meter.clone();
        if let Some(account) = &self.shared {
            total.absorb(&account.others(&self.prices, self.meter.fundings()));
        }
        total
    }

    /// The ledger's view for `/budget`: spend, reservations in flight, the day.
    pub fn overview(&self) -> Overview {
        match &self.shared {
            Some(account) => account.overview(&self.prices, self.meter.fundings()),
            None => Overview { session: self.meter.clone(), ..Overview::default() },
        }
    }

    /// Every setting file of the user's state folder and the project. A missing file is
    /// the default; a broken line is kept in `problems` and skipped. `JEV_LOCAL_ONLY=1`
    /// turns local-only mode on whatever the files say.
    pub fn load(host: &dyn Host, audit: Arc<dyn PrivacyAudit>, state_dir: Option<&Path>, project: &Path) -> Self {
        let (privacy, privacy_errors) = super::privacy_files::load(host, state_dir, project);
        let (rules, rule_errors) = jev_routing::application::rule_files::load(state_dir, project);
        let local_only = host.env("JEV_LOCAL_ONLY").is_some_and(|v| matches!(v.trim(), "1" | "on" | "true" | "yes"));
        let mut problems: Vec<String> = privacy_errors.iter().map(|(file, e)| format!("{}: {e}", file.display())).collect();
        problems.extend(rule_errors.iter().map(|e| format!("routing.rules line {}: {}", e.line, e.message)));
        Self {
            home: host.home(),
            privacy: privacy.with_local_only(local_only),
            audit: state_dir.map(|dir| AuditLog::in_state_dir(dir, audit)),
            rules,
            budget: super::guard_store::load_budget(host, state_dir),
            prices: super::guard_store::load_prices(host, state_dir),
            failover: super::guard_store::load_failover(host, state_dir),
            problems,
            ..Self::default()
        }
    }

    /// Narrow `usable` for this turn: privacy policy, budget cap, failover policy, then the
    /// user's routing rules. `Err` is a plain sentence when a step that started with models
    /// left none.
    pub fn restrict(&self, usable: &Usable, turn: &TurnFacts) -> Result<Restricted, String> {
        let verdict = self.verdict(turn);
        let blocked: Vec<(ProviderId, String)> =
            jev_shared::ALL_PROVIDERS.into_iter().filter(|p| usable.has_provider(*p)).filter_map(|p| verdict.denial(p).map(|why| (p, why))).collect();
        let mut current = usable.filter(|p, _| verdict.permits(p));
        if current.is_empty() && !usable.is_empty() {
            return Err(verdict.refusal_line());
        }
        let may_pay = self.may_pay(turn.attempts);
        if !may_pay {
            let free = current.filter(|p, m| !self.meter.is_paid(p, m));
            if free.is_empty() && !current.is_empty() {
                let why = if !self.budget.allows_paid(&self.total_meter()) {
                    "the budget cap is reached (/budget)"
                } else if !self.allows_paid_today() {
                    "the daily budget cap is reached (/budget)"
                } else {
                    "paid-fallback is off (/failover)"
                };
                return Err(format!("only models that cost money are left, and {why}"));
            }
            current = free;
        }
        let mut notes = Vec::new();
        if !turn.pinned && !self.rules.is_empty() {
            let facts = routing_rules::Facts::new(turn.root, turn.prompt, turn.task, self.home.as_deref());
            let applied = self.rules.apply(&current, &facts)?;
            current = applied.usable;
            notes = applied.notes;
        }
        Ok(Restricted { usable: current, notes, blocked })
    }

    /// The privacy policy's verdict on this turn: the prompt, and for each provider that is
    /// behind, the catch-up it would get with it.
    pub fn verdict(&self, turn: &TurnFacts) -> Verdict {
        let mut verdict = self.privacy.check(turn.root, turn.prompt, turn.attached, self.home.as_deref());
        for (provider, text) in turn.catch_ups {
            let theirs = self.privacy.check(turn.root, &format!("{text}\n{}", turn.prompt), turn.attached, self.home.as_deref());
            verdict.take_denial(*provider, &theirs);
        }
        verdict
    }

    /// The privacy policy alone, for a turn that goes to `provider` whatever routing says
    /// (routing off: Claude only). `Err` is the refusal sentence; the refusal is audited.
    pub fn privacy_allows(&self, provider: ProviderId, turn: &TurnFacts, now_ms: u64) -> Result<(), String> {
        let verdict = self.verdict(turn);
        match verdict.denial(provider) {
            None => Ok(()),
            Some(why) => {
                if turn.attempts == 0 {
                    self.audit_blocked(&[(provider, why)], now_ms);
                }
                Err(verdict.refusal_line())
            }
        }
    }

    /// Whether the policy lets this text leave the machine at all (some cloud provider may
    /// have it). The Jev classifier is a cloud service too, so it is not asked otherwise.
    pub fn may_leave(&self, root: &Path, text: &str, attached: &[std::path::PathBuf]) -> bool {
        use crate::domain::privacy::{locality, Locality};
        let verdict = self.privacy.check(root, text, attached, self.home.as_deref());
        jev_shared::ALL_PROVIDERS.into_iter().filter(|p| locality(*p) == Locality::Cloud).any(|p| verdict.permits(p))
    }

    /// The prompt as it may leave for `provider`, after the secret scan; the audit log
    /// gets a line saying what was sent.
    pub fn outgoing(&self, prompt: &str, provider: ProviderId, model: &str, now_ms: u64) -> Outgoing {
        let out = outgoing::prepare(&self.privacy, prompt);
        if let Some(log) = &self.audit {
            log.record(&AuditRecord::sent(now_ms, provider, model, &out));
        }
        out
    }

    /// A message the person typed while a turn runs on `provider`, as it may be added to that
    /// turn: `None` when the privacy policy does not let this text go to `provider` (it then
    /// waits as a turn of its own, where routing decides), else with its secrets blanked and
    /// the audit log told, as for a prompt.
    pub fn steer_text(&self, root: &Path, text: &str, provider: ProviderId, model: &str, now_ms: u64) -> Option<String> {
        self.privacy.check(root, text, &[], self.home.as_deref()).permits(provider).then(|| self.outgoing(text, provider, model, now_ms).prompt)
    }

    /// Whether the privacy policy lets `text` go to `provider` (nothing is audited or changed).
    pub fn text_permitted(&self, root: &Path, text: &str, provider: ProviderId) -> bool {
        self.privacy.check(root, text, &[], self.home.as_deref()).permits(provider)
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
    use crate::domain::budget::Cap;
    use crate::domain::privacy::Source;

    fn facts<'a>(prompt: &'a str, attempts: u32) -> TurnFacts<'a> {
        TurnFacts { root: Path::new("/w"), prompt, catch_ups: &[], task: None, attached: &[], pinned: false, attempts }
    }

    fn all() -> Usable {
        Usable::all_of(jev_shared::ALL_PROVIDERS)
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
        assert!(jev_shared::ALL_PROVIDERS.iter().all(|p| has(&r.usable, *p)));
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
        let mut g = Guards { budget: Budget { cap: Some(Cap::Tokens(100)), ..Budget::default() }, ..Guards::default() };
        let usage = jev_backend::Usage { input_tokens: 200, ..Default::default() };
        g.meter.record(ProviderId::DeepSeek, "deepseek-v4-pro", &usage, &g.prices);
        let r = g.restrict(&all(), &facts("x", 0)).unwrap();
        assert!(!has(&r.usable, ProviderId::DeepSeek) && has(&r.usable, ProviderId::Claude));
        // Only paid models left: refused with the reason.
        let err = g.restrict(&Usable::all_of([ProviderId::DeepSeek]), &facts("x", 0)).unwrap_err();
        assert!(err.contains("budget cap is reached"), "{err}");
    }

    /// The inspection's reproduction: Codex configured with a custom model provider is
    /// controlled like paid use, so the cap and the paid-fallback switch apply to it.
    #[test]
    fn a_custom_codex_backend_is_held_by_the_cap_and_the_failover_switch() {
        use jev_providers::domain::funding::{Funding, Fundings};
        let mut g = Guards { budget: Budget { cap: Some(Cap::Tokens(100)), ..Budget::default() }, ..Guards::default() };
        let codex_only = Usable::all_of([ProviderId::Codex]);
        assert!(g.restrict(&codex_only, &facts("x", 0)).is_ok());
        g.set_fundings(Fundings::default().with(ProviderId::Codex, Funding::Custom));
        let usage = jev_backend::Usage { input_tokens: 200, ..Default::default() };
        g.meter.record(ProviderId::Codex, "gpt-6.1-terra", &usage, &g.prices);
        let r = g.restrict(&all(), &facts("x", 0)).unwrap();
        assert!(!has(&r.usable, ProviderId::Codex) && has(&r.usable, ProviderId::Claude), "capped once its tokens reach the cap");
        assert!(g.restrict(&codex_only, &facts("x", 0)).unwrap_err().contains("budget cap is reached"));
        let off = Guards { failover: FailoverPolicy { max_attempts: 6, paid_fallback: false }, ..Guards::default() };
        let mut off = off;
        off.set_fundings(Fundings::default().with(ProviderId::Codex, Funding::Custom));
        assert!(!has(&off.restrict(&all(), &facts("x", 1)).unwrap().usable, ProviderId::Codex));
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
    fn a_path_named_in_an_earlier_turn_still_holds_a_provider_back() {
        let g = guards("block secrets/** from codex");
        // The new prompt names nothing, but the catch-up Codex would get does.
        let behind = [(ProviderId::Codex, "look at secrets/prod.yml\nit sets the key".to_string())];
        let later = TurnFacts { catch_ups: &behind, ..facts("continue", 0) };
        let r = g.restrict(&all(), &later).unwrap();
        assert!(!has(&r.usable, ProviderId::Codex) && has(&r.usable, ProviderId::Claude));
        // Without that catch-up nothing is held back.
        assert!(has(&g.restrict(&all(), &facts("continue", 0)).unwrap().usable, ProviderId::Codex));
    }

    #[test]
    fn a_catch_up_only_holds_back_the_provider_it_goes_to() {
        let g = guards("only claude for secrets/**");
        // Gemini is behind and its catch-up (git status included) names the path; Codex is current.
        let behind = [(ProviderId::Gemini, "git status:\n M secrets/prod.yml".to_string())];
        let r = g.restrict(&all(), &TurnFacts { catch_ups: &behind, ..facts("continue", 0) }).unwrap();
        assert!(!has(&r.usable, ProviderId::Gemini), "the catch-up names a covered path");
        assert!(has(&r.usable, ProviderId::Codex), "a provider that is current gets only the prompt");
        assert!(has(&r.usable, ProviderId::Claude));
    }

    #[test]
    fn outgoing_blanks_secrets_and_audits_without_the_secret() {
        let d = std::env::temp_dir().join(format!("jev-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let g = Guards { audit: Some(AuditLog::in_state_dir(&d, Arc::new(crate::infrastructure::privacy_audit::FilePrivacyAudit))), ..Guards::default() };
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
        let g = Guards::load(&jev_shared::infrastructure::system_host::SystemHost, Arc::new(crate::infrastructure::privacy_audit::FilePrivacyAudit), Some(&state), &project);
        assert_eq!(g.privacy.rule_count(), 1);
        assert_eq!(g.problems.len(), 1);
        assert_eq!(g.budget.cap, Some(Cap::Usd(3.0)));
        assert_eq!(g.failover.max_attempts, 2);
        assert!(!g.rules.is_empty() && g.audit.is_some());
        let _ = std::fs::remove_dir_all(&base);
    }
}
