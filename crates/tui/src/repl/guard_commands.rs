//! The session commands that show and change the guards: `/budget`, `/privacy`, `/failover`
//! and `/scoreboard`. Parsing is pure; running one changes the [`Guards`] and says what happened.

use std::path::Path;

use jev_turn::domain::{budget, budget_report};
use jev_turn::domain::budget::Cap;
use jev_turn::application::guards::Guards;
use jev_shared::infrastructure::system_host::SystemHost;

#[derive(Debug, Clone, PartialEq)]
pub enum GuardCmd {
    /// `/budget`: the meter and the cap.
    BudgetShow,
    /// `/budget usd 5 | tokens 2m`.
    BudgetSet(Cap),
    /// `/budget off`.
    BudgetClear,
    /// `/budget daily usd 20 | tokens 5m | off`: the paid cap of one day over every session.
    BudgetDaily(Option<Cap>),
    /// `/budget allowance 5m | off`: the soft subscription allowance per provider and session.
    BudgetAllowance(Option<u64>),
    /// `/privacy`: the policy, the rules and the problems found in them.
    PrivacyShow,
    /// `/privacy local-only on|off`, for this session.
    LocalOnly(bool),
    /// `/privacy audit`: the last lines of the audit log.
    Audit,
    /// `/failover`.
    FailoverShow,
    /// `/failover <setting> <value>`.
    FailoverSet { key: String, value: String },
    /// `/scoreboard`: how each provider has done this session.
    Health,
}

const AUDIT_LINES: usize = 20;

impl GuardCmd {
    /// `head` is the command word, `args` the rest of the line.
    pub fn parse(head: &str, args: &str) -> Result<Self, String> {
        let args = args.trim();
        let words: Vec<&str> = args.split_whitespace().collect();
        match (head, words.as_slice()) {
            ("/budget", []) => Ok(Self::BudgetShow),
            ("/budget", ["off" | "none"]) => Ok(Self::BudgetClear),
            ("/budget", ["daily", "off" | "none"]) => Ok(Self::BudgetDaily(None)),
            ("/budget", ["daily", rest @ ..]) => Cap::parse(&rest.join(" ")).map(|cap| Self::BudgetDaily(Some(cap))),
            ("/budget", ["allowance", "off" | "none"]) => Ok(Self::BudgetAllowance(None)),
            ("/budget", ["allowance", amount]) => budget::parse_tokens(amount).map(|t| Self::BudgetAllowance(Some(t))).ok_or_else(|| "usage: /budget allowance <tokens, like 5m> | off".to_string()),
            ("/budget", _) => Cap::parse(args).map(Self::BudgetSet),
            ("/privacy", []) => Ok(Self::PrivacyShow),
            ("/privacy", ["audit"]) => Ok(Self::Audit),
            ("/privacy", ["local-only", "on"]) => Ok(Self::LocalOnly(true)),
            ("/privacy", ["local-only", "off"]) => Ok(Self::LocalOnly(false)),
            ("/privacy", _) => Err("usage: /privacy [audit | local-only on|off]".into()),
            ("/failover", []) => Ok(Self::FailoverShow),
            ("/failover", [key, value]) => Ok(Self::FailoverSet { key: (*key).into(), value: (*value).into() }),
            ("/failover", _) => Err("usage: /failover [max-attempts <n> | paid-fallback on|off]".into()),
            ("/scoreboard", []) => Ok(Self::Health),
            _ => Err(format!("usage: {head}")),
        }
    }
}

/// Run `cmd`; `plans` are the subscriptions' use of their limits. Settings that outlive the
/// session are saved under `state_dir` when there is one.
pub fn run(cmd: GuardCmd, guards: &mut Guards, state_dir: Option<&Path>, plans: &[(String, f64)]) -> String {
    match cmd {
        GuardCmd::BudgetShow => {
            let total = guards.total_meter();
            let overview = guards.overview();
            let limits = budget_report::limits_text(&guards.budget, &total, &overview.session_reserved, &overview.day);
            format!("{}\n{limits}", budget::meter_text(&total, &guards.budget, plans))
        }
        GuardCmd::BudgetSet(cap) => {
            guards.budget.cap = Some(cap);
            saved(state_dir, |d| jev_turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), "Paid models stop being picked once the cap is used up.")
        }
        GuardCmd::BudgetDaily(cap) => {
            guards.budget.daily = cap;
            let done = if cap.is_some() { "Paid models stop being admitted once the day's paid use over every session reaches the cap." } else { "No daily cap on paid use." };
            saved(state_dir, |d| jev_turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), done)
        }
        GuardCmd::BudgetAllowance(tokens) => {
            guards.budget.allowance = tokens.map(|tokens| budget::Allowance { tokens });
            let done = if tokens.is_some() { "A subscription provider past the allowance is warned about; it is never refused." } else { "No subscription allowance." };
            saved(state_dir, |d| jev_turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), done)
        }
        GuardCmd::BudgetClear => {
            guards.budget.cap = None;
            saved(state_dir, |d| jev_turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), "No cap on paid use.")
        }
        GuardCmd::PrivacyShow => privacy_text(guards),
        GuardCmd::LocalOnly(on) => match guards.privacy.set_local_only(on) {
            Err(why) => format!("  {why}"),
            Ok(()) if on => "  Local-only mode is on for this session: no prompt goes to a cloud provider.".into(),
            Ok(()) => "  Local-only mode is off.".into(),
        },
        GuardCmd::Audit => match &guards.audit {
            None => "  No audit log: this session does not keep state.".into(),
            Some(log) => {
                let recs = log.tail(AUDIT_LINES);
                if recs.is_empty() {
                    format!("  Nothing recorded yet in {}", log.path().display())
                } else {
                    recs.iter().map(|r| format!("  {r}")).collect::<Vec<_>>().join("\n")
                }
            }
        },
        GuardCmd::FailoverShow => format!(
            "  max-attempts {}  (how many models one turn may try)\n  paid-fallback {}  (a failover may land on a model that costs money)",
            guards.failover.max_attempts,
            if guards.failover.paid_fallback { "on" } else { "off" }
        ),
        GuardCmd::FailoverSet { key, value } => match guards.failover.set(&key, &value) {
            Ok(()) => saved(state_dir, |d| jev_turn::application::guard_store::save_failover(&SystemHost, &guards.failover, d), "Failover policy changed."),
            Err(e) => format!("  {e}"),
        },
        GuardCmd::Health => {
            if guards.scoreboard.is_empty() {
                "  No provider has answered yet in this session.".into()
            } else {
                guards.scoreboard.text()
            }
        }
    }
}

fn saved(state_dir: Option<&Path>, save: impl FnOnce(&Path) -> std::io::Result<()>, done: &str) -> String {
    match state_dir.map(save) {
        Some(Err(e)) => format!("  {done} (could not be kept for next time: {e})"),
        _ => format!("  {done}"),
    }
}

fn privacy_text(g: &Guards) -> String {
    let p = &g.privacy;
    let mut out = format!(
        "  Local-only: {}\n  Secret scan: {}\n  Privacy rules: {}\n  Routing rules: {}\n",
        if p.local_only() { "on" } else { "off" },
        if p.redacts() { "on (secrets are blanked before a prompt leaves)" } else { "off" },
        p.rule_count(),
        g.rules.lines().count()
    );
    for line in g.rules.lines() {
        out.push_str(&format!("    {line}\n"));
    }
    for problem in &g.problems {
        out.push_str(&format!("  Could not read: {problem}\n"));
    }
    out.push_str("  See what was sent: /privacy audit");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_spelling() {
        assert_eq!(GuardCmd::parse("/budget", ""), Ok(GuardCmd::BudgetShow));
        assert_eq!(GuardCmd::parse("/budget", "off"), Ok(GuardCmd::BudgetClear));
        assert_eq!(GuardCmd::parse("/budget", "usd 5"), Ok(GuardCmd::BudgetSet(Cap::Usd(5.0))));
        assert!(GuardCmd::parse("/budget", "lots").is_err());
        assert_eq!(GuardCmd::parse("/budget", "daily usd 20"), Ok(GuardCmd::BudgetDaily(Some(Cap::Usd(20.0)))));
        assert_eq!(GuardCmd::parse("/budget", "daily off"), Ok(GuardCmd::BudgetDaily(None)));
        assert_eq!(GuardCmd::parse("/budget", "allowance 5m"), Ok(GuardCmd::BudgetAllowance(Some(5_000_000))));
        assert_eq!(GuardCmd::parse("/budget", "allowance off"), Ok(GuardCmd::BudgetAllowance(None)));
        assert!(GuardCmd::parse("/budget", "allowance lots").is_err());
        assert!(GuardCmd::parse("/budget", "daily lots").is_err());
        assert_eq!(GuardCmd::parse("/privacy", "local-only on"), Ok(GuardCmd::LocalOnly(true)));
        assert_eq!(GuardCmd::parse("/privacy", "audit"), Ok(GuardCmd::Audit));
        assert!(GuardCmd::parse("/privacy", "x y z").is_err());
        assert_eq!(GuardCmd::parse("/failover", "max-attempts 2"), Ok(GuardCmd::FailoverSet { key: "max-attempts".into(), value: "2".into() }));
        assert_eq!(GuardCmd::parse("/scoreboard", ""), Ok(GuardCmd::Health));
    }

    #[test]
    fn a_cap_is_kept_and_cleared_on_disk() {
        let dir = std::env::temp_dir().join(format!("jev-guardcmd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut g = Guards::default();
        run(GuardCmd::BudgetSet(Cap::Usd(3.0)), &mut g, Some(&dir), &[]);
        assert_eq!(Guards::load(&SystemHost, std::sync::Arc::new(jev_turn::infrastructure::privacy_audit::FilePrivacyAudit), Some(&dir), &dir).budget.cap, Some(Cap::Usd(3.0)));
        run(GuardCmd::BudgetClear, &mut g, Some(&dir), &[]);
        assert_eq!(Guards::load(&SystemHost, std::sync::Arc::new(jev_turn::infrastructure::privacy_audit::FilePrivacyAudit), Some(&dir), &dir).budget.cap, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn local_only_toggles_and_failover_rejects_bad_values() {
        let mut g = Guards::default();
        run(GuardCmd::LocalOnly(true), &mut g, None, &[]);
        assert!(g.privacy.local_only());
        run(GuardCmd::LocalOnly(false), &mut g, None, &[]);
        assert!(!g.privacy.local_only());
        let said = run(GuardCmd::FailoverSet { key: "max-attempts".into(), value: "99".into() }, &mut g, None, &[]);
        assert!(said.contains("between 1 and"), "{said}");
        run(GuardCmd::FailoverSet { key: "paid-fallback".into(), value: "off".into() }, &mut g, None, &[]);
        assert!(!g.failover.paid_fallback);
    }
}
