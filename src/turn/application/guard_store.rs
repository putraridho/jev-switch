//! Keeping the budget cap and the failover policy between sessions: the writing half of
//! `Budget::load` and `FailoverPolicy::load`.

use std::path::Path;

use crate::shared::application::host;
use crate::turn::domain::budget::Budget;
use crate::turn::domain::failover_policy::FailoverPolicy;

/// Keep the cap (or no cap) for the next session.
pub fn save_budget(budget: &Budget, state_dir: &Path) -> std::io::Result<()> {
    let path = Budget::file_path(state_dir);
    match budget.file_text() {
        Some(text) => host::write_private(&path, &text),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Keep the failover policy for the next session.
pub fn save_failover(policy: &FailoverPolicy, state_dir: &Path) -> std::io::Result<()> {
    host::write_private(&FailoverPolicy::file_path(state_dir), &policy.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::domain::budget::{Cap, PriceTable};

    #[test]
    fn the_cap_is_kept_between_sessions() {
        let d = std::env::temp_dir().join(format!("jev-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(Budget::load(Some(&d)), Budget::default());
        for cap in [Cap::Usd(5.0), Cap::Tokens(2_000_000)] {
            save_budget(&Budget { cap: Some(cap) }, &d).unwrap();
            assert_eq!(Budget::load(Some(&d)).cap, Some(cap));
        }
        save_budget(&Budget::default(), &d).unwrap();
        assert_eq!(Budget::load(Some(&d)).cap, None);
        save_budget(&Budget::default(), &d).unwrap();
        std::fs::write(d.join("prices.txt"), "m 1 1 1").unwrap();
        assert!(PriceTable::load(Some(&d)).price("m").is_some());
        let _ = std::fs::remove_dir_all(&d);
    }


    #[test]
    fn the_policy_is_kept() {
        let d = std::env::temp_dir().join(format!("jev-failover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(FailoverPolicy::load(Some(&d)), FailoverPolicy::default());
        let p = FailoverPolicy { max_attempts: 2, paid_fallback: false };
        save_failover(&p, &d).unwrap();
        assert_eq!(FailoverPolicy::load(Some(&d)), p);
        let _ = std::fs::remove_dir_all(&d);
    }
}
