//! Keeping the budget cap, prices and failover policy between sessions.

use std::path::Path;

use jev_shared::application::host::Host;
use crate::domain::budget::{Budget, Cap, PriceTable};
use crate::domain::failover_policy::FailoverPolicy;

/// The cap kept from an earlier session; no cap when there is no readable file.
pub fn load_budget(host: &dyn Host, state_dir: Option<&Path>) -> Budget {
    let cap = state_dir.and_then(|d| host.read_text(&Budget::file_path(d)).ok()).and_then(|t| Cap::parse(&t).ok());
    Budget { cap }
}

/// The user's own prices; the built-in ones when there is no readable file.
pub fn load_prices(host: &dyn Host, state_dir: Option<&Path>) -> PriceTable {
    let text = state_dir.and_then(|d| host.read_text(&PriceTable::file_path(d)).ok()).unwrap_or_default();
    PriceTable::parse(&text).0
}

/// The failover policy kept from an earlier session, else the default.
pub fn load_failover(host: &dyn Host, state_dir: Option<&Path>) -> FailoverPolicy {
    state_dir.and_then(|d| host.read_text(&FailoverPolicy::file_path(d)).ok()).map(|t| FailoverPolicy::parse(&t)).unwrap_or_default()
}

/// Keep the cap (or no cap) for the next session.
pub fn save_budget(host: &dyn Host, budget: &Budget, state_dir: &Path) -> std::io::Result<()> {
    let path = Budget::file_path(state_dir);
    match budget.file_text() {
        Some(text) => host.write_private(&path, &text),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Keep the failover policy for the next session.
pub fn save_failover(host: &dyn Host, policy: &FailoverPolicy, state_dir: &Path) -> std::io::Result<()> {
    host.write_private(&FailoverPolicy::file_path(state_dir), &policy.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::infrastructure::system_host::SystemHost;

    #[test]
    fn the_cap_is_kept_between_sessions() {
        let d = std::env::temp_dir().join(format!("jev-budget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(load_budget(&SystemHost, Some(&d)), Budget::default());
        for cap in [Cap::Usd(5.0), Cap::Tokens(2_000_000)] {
            save_budget(&SystemHost, &Budget { cap: Some(cap) }, &d).unwrap();
            assert_eq!(load_budget(&SystemHost, Some(&d)).cap, Some(cap));
        }
        save_budget(&SystemHost, &Budget::default(), &d).unwrap();
        assert_eq!(load_budget(&SystemHost, Some(&d)).cap, None);
        save_budget(&SystemHost, &Budget::default(), &d).unwrap();
        std::fs::write(d.join("prices.txt"), "m 1 1 1").unwrap();
        assert!(load_prices(&SystemHost, Some(&d)).price("m").is_some());
        let _ = std::fs::remove_dir_all(&d);
    }


    #[test]
    fn the_policy_is_kept() {
        let d = std::env::temp_dir().join(format!("jev-failover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(load_failover(&SystemHost, Some(&d)), FailoverPolicy::default());
        let p = FailoverPolicy { max_attempts: 2, paid_fallback: false };
        save_failover(&SystemHost, &p, &d).unwrap();
        assert_eq!(load_failover(&SystemHost, Some(&d)), p);
        let _ = std::fs::remove_dir_all(&d);
    }
}
