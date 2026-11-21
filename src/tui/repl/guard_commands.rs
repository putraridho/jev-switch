//! The session commands that show and change the guards: `/budget`, `/privacy`, `/failover`
//! and `/scoreboard`. Parsing is pure; running one changes the [`Guards`] and says what happened.

use std::path::Path;

use crate::turn::domain::budget;
use crate::turn::domain::budget::Cap;
use crate::turn::application::guards::Guards;
use crate::shared::infrastructure::system_host::SystemHost;

#[derive(Debug, Clone, PartialEq)]
pub enum GuardCmd {
    /// `/budget`: the meter and the cap.
    BudgetShow,
    /// `/budget usd 5 | tokens 2m`.
    BudgetSet(Cap),
    /// `/budget off`.
    BudgetClear,
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
        GuardCmd::BudgetShow => budget::meter_text(&guards.meter, &guards.budget, plans),
        GuardCmd::BudgetSet(cap) => {
            guards.budget.cap = Some(cap);
            saved(state_dir, |d| crate::turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), "Paid models stop being picked once the cap is used up.")
        }
        GuardCmd::BudgetClear => {
            guards.budget.cap = None;
            saved(state_dir, |d| crate::turn::application::guard_store::save_budget(&SystemHost, &guards.budget, d), "No cap on paid use.")
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
            Ok(()) => saved(state_dir, |d| crate::turn::application::guard_store::save_failover(&SystemHost, &guards.failover, d), "Failover policy changed."),
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
        assert_eq!(Guards::load(&SystemHost, Some(&dir), &dir).budget.cap, Some(Cap::Usd(3.0)));
        run(GuardCmd::BudgetClear, &mut g, Some(&dir), &[]);
        assert_eq!(Guards::load(&SystemHost, Some(&dir), &dir).budget.cap, None);
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
