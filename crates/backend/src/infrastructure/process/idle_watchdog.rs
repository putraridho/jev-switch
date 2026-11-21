//! How long a CLI may stay silent.

use std::time::Duration;

/// Longest silence from a CLI before its turn is cut off, so a hung CLI cannot
/// freeze the session (`CODE_IDLE_TIMEOUT_SECS`, default 900).
pub fn idle_timeout_from_env() -> Duration {
    idle_timeout(jev_shared::infrastructure::env_file::lookup("CODE_IDLE_TIMEOUT_SECS").as_deref())
}

/// The pure half of [`idle_timeout_from_env`]: seconds as text; anything that is not a
/// positive number leaves the default.
pub fn idle_timeout(value: Option<&str>) -> Duration {
    let secs = value.and_then(|s| s.trim().parse::<u64>().ok()).filter(|s| *s > 0).unwrap_or(900);
    Duration::from_secs(secs)
}
