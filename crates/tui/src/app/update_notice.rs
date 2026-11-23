//! The passive update check: at the start of an interactive session, tell the person when a
//! newer jev-code is waiting, so they know to run `/update`.
//!
//! It reuses [`self_update::check`] (a `git fetch` of the checkout's upstream) on a background
//! thread, so the banner never waits for the network. The network is asked at most once a day:
//! the answer is kept in `<state dir>/update-check`, and a newer version found earlier is
//! announced again from there at every start until it is installed. Nothing is installed or
//! changed, and a failed check (offline, no upstream, not a checkout) says nothing.
//! `JEV_UPDATE_CHECK=off` turns it off.

use std::path::{Path, PathBuf};

use crate::app::self_update::{self, Facts};
use crate::console::terminal_ui::Console;

/// The cache file in the state directory.
const CACHE_FILE: &str = "update-check";

/// How long an answer is trusted before the network is asked again.
pub const CHECK_EVERY_MS: u64 = 24 * 60 * 60 * 1000;

/// What the last check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    /// Unix time in milliseconds of the check.
    pub checked_ms: u64,
    /// The newer version it found, if any.
    pub newer: Option<String>,
}

impl Cache {
    /// `<checked_ms>\n<newer version or empty>\n`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Cache> {
        let mut lines = text.lines();
        let checked_ms = lines.next()?.trim().parse().ok()?;
        let newer = lines.next().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        Some(Cache { checked_ms, newer })
    }

    pub fn render(&self) -> String {
        format!("{}\n{}\n", self.checked_ms, self.newer.as_deref().unwrap_or(""))
    }
}

/// Whether the setting leaves the check on: `off`, `0`, `false` and `no` turn it off.
pub fn enabled(setting: Option<&str>) -> bool {
    !matches!(setting.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("off" | "0" | "false" | "no"))
}

/// Whether the network should be asked now.
pub fn is_due(cache: Option<&Cache>, now_ms: u64) -> bool {
    cache.is_none_or(|c| now_ms.saturating_sub(c.checked_ms) >= CHECK_EVERY_MS)
}

/// The numeric parts of a version (`1.11.12-rc.1` → 1, 11, 12); a missing part counts as 0.
fn parts(version: &str) -> Vec<u64> {
    version.split(['.', '-', '+']).map_while(|p| p.parse().ok()).collect()
}

/// Whether `candidate` is a higher version than `running`. An unreadable version is never newer.
pub fn is_newer(candidate: &str, running: &str) -> bool {
    let (a, b) = (parts(candidate), parts(running));
    if a.is_empty() {
        return false;
    }
    let len = a.len().max(b.len());
    let at = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..len).map(|i| (at(&a, i), at(&b, i))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

/// The newer version the facts show: on the upstream (not yet pulled) or in the checkout
/// (pulled, not yet installed). New commits under the same version are not announced.
pub fn newer_version(facts: &Facts, running: &str) -> Option<String> {
    let upstream = if facts.behind > 0 { facts.upstream_version.as_deref() } else { None };
    [upstream, facts.local_version.as_deref()].into_iter().flatten().filter(|v| is_newer(v, running)).max_by_key(|v| parts(v)).map(str::to_string)
}

/// The line shown to the person.
pub fn notice(newer: &str, running: &str) -> String {
    format!("jev-code {newer} is available (you have {running}): run /update")
}

/// ssh must not ask for a passphrase on the session's terminal. Left alone when the person
/// set their own ssh command, in the environment or in git's config.
fn quiet_ssh(dir: &Path) -> Vec<(&'static str, &'static str)> {
    let own = std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some();
    let configured = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get", "core.sshCommand"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    if own || configured { Vec::new() } else { vec![("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")] }
}

fn cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join(CACHE_FILE)
}

fn read_cache(state_dir: &Path) -> Option<Cache> {
    Cache::parse(&std::fs::read_to_string(cache_path(state_dir)).ok()?)
}

/// Announce a version found earlier and, when a day has passed, ask the network on a
/// background thread. Returns at once. Notices go through the console, which holds them for
/// the prompt, so they never land inside a turn's output or the line being typed.
pub fn start(state_dir: Option<&Path>, console: &Console, now_ms: u64) {
    if !enabled(jev_shared::infrastructure::env_file::lookup("JEV_UPDATE_CHECK").as_deref()) {
        return;
    }
    let (Some(state_dir), Ok(dir)) = (state_dir, self_update::source_dir()) else { return };
    let running = self_update::running_version();
    let cache = read_cache(state_dir);
    let announced = cache.as_ref().and_then(|c| c.newer.clone()).filter(|v| is_newer(v, running));
    if let Some(newer) = &announced {
        console.notify(&notice(newer, running));
    }
    if !is_due(cache.as_ref(), now_ms) {
        return;
    }
    let (console, state_dir) = (console.clone(), state_dir.to_path_buf());
    std::thread::spawn(move || {
        let env = quiet_ssh(&dir);
        let Ok(facts) = self_update::check_with(&dir, &env) else { return };
        let newer = newer_version(&facts, running);
        let _ = jev_shared::infrastructure::private_file::write_atomic(&cache_path(&state_dir), &Cache { checked_ms: now_ms, newer: newer.clone() }.render());
        if let Some(newer) = newer.filter(|n| announced.as_ref() != Some(n)) {
            console.notify(&notice(&newer, running));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(behind: usize, upstream: &str, local: &str) -> Facts {
        Facts {
            upstream: Some("origin/main".into()),
            changed: Vec::new(),
            ahead: 0,
            behind,
            incoming: Vec::new(),
            upstream_version: Some(upstream.into()),
            local_version: Some(local.into()),
        }
    }

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(is_newer("1.11.13", "1.11.12"));
        assert!(is_newer("1.12.0", "1.11.99"));
        assert!(is_newer("1.10.0", "1.9.3"), "10 is more than 9");
        assert!(is_newer("2.0", "1.99.9"));
        assert!(!is_newer("1.11.12", "1.11.12"));
        assert!(!is_newer("1.9.3", "1.11.12"));
        assert!(!is_newer("?", "1.11.12"), "an unreadable version is not an update");
    }

    #[test]
    fn a_newer_upstream_or_checkout_is_announced() {
        assert_eq!(newer_version(&facts(2, "1.12.0", "1.11.12"), "1.11.12"), Some("1.12.0".into()));
        assert_eq!(newer_version(&facts(0, "1.11.12", "1.12.0"), "1.11.12"), Some("1.12.0".into()), "pulled by hand, not installed");
        assert_eq!(newer_version(&facts(1, "1.11.12", "1.11.12"), "1.11.12"), None, "new commits, same version");
        assert_eq!(newer_version(&facts(0, "1.11.12", "1.11.12"), "1.11.12"), None);
        assert_eq!(newer_version(&facts(0, "1.13.0", "1.11.12"), "1.11.12"), None, "an upstream we are not behind is not news");
    }

    #[test]
    fn the_cache_round_trips_and_ignores_garbage() {
        let found = Cache { checked_ms: 1_700_000_000_000, newer: Some("1.12.0".into()) };
        assert_eq!(Cache::parse(&found.render()), Some(found));
        let none = Cache { checked_ms: 5, newer: None };
        assert_eq!(Cache::parse(&none.render()), Some(none));
        assert_eq!(Cache::parse("not a time\n1.0.0\n"), None);
        assert_eq!(Cache::parse(""), None);
    }

    #[test]
    fn the_network_is_asked_once_a_day() {
        let at = |ms| Some(Cache { checked_ms: ms, newer: None });
        assert!(is_due(None, 1_000));
        assert!(!is_due(at(1_000).as_ref(), 1_000 + CHECK_EVERY_MS - 1));
        assert!(is_due(at(1_000).as_ref(), 1_000 + CHECK_EVERY_MS));
        assert!(!is_due(at(5_000).as_ref(), 1_000), "a clock that went back does not trigger a check");
    }

    #[test]
    fn the_setting_turns_it_off() {
        assert!(enabled(None) && enabled(Some("on")) && enabled(Some("")));
        assert!(!enabled(Some("off")) && !enabled(Some(" OFF ")) && !enabled(Some("0")) && !enabled(Some("false")));
    }

    #[test]
    fn the_notice_names_both_versions_and_the_command() {
        assert_eq!(notice("1.12.0", "1.11.12"), "jev-code 1.12.0 is available (you have 1.11.12): run /update");
    }
}
