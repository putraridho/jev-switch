//! Crash recovery of the turn that was running: while a turn runs, a small marker file in
//! the state folder names it (`inflight-<pid>.json`); a turn that ends, however it ends,
//! removes it. A marker whose process is gone is a turn that never got to finish (a crash,
//! a power cut, `kill -9`), and the next `jev-code` in that folder says so and shows the
//! prompt, so it is not lost.
//!
//! The marker keeps only the redacted first line of the prompt (see [`crate::shared::domain::audit::preview`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::shared::domain::utc::utc_stamp;

const PREFIX: &str = "inflight-";

/// What a running turn leaves behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub pid: u32,
    pub started_ms: u64,
    pub project: String,
    pub prompt: String,
}

fn marker_path(state_dir: &Path, pid: u32) -> PathBuf {
    state_dir.join(format!("{PREFIX}{pid}.json"))
}

/// Note that a turn started. Failing to write is not worth a lost turn, so it is ignored.
pub fn begin(state_dir: &Path, project: &Path, prompt: &str, now_ms: u64) {
    let marker = Marker { pid: std::process::id(), started_ms: now_ms, project: project.to_string_lossy().into_owned(), prompt: crate::shared::domain::audit::preview(prompt) };
    if let Ok(text) = serde_json::to_string(&marker) {
        let _ = crate::shared::infrastructure::private_file::write_atomic(&marker_path(state_dir, marker.pid), &text);
    }
}

/// Note that the turn of this process ended.
pub fn end(state_dir: &Path) {
    let _ = std::fs::remove_file(marker_path(state_dir, std::process::id()));
}

/// Whether a process with this id exists. On platforms where that cannot be asked without a
/// new dependency every process counts as alive, so nothing is ever reported as crashed there.
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else { return true };
        // Signal 0 only checks; EPERM means it exists and belongs to someone else.
        unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Whether a process that has run for `elapsed_secs` started after the marker was written:
/// then the pid belongs to a newer, unrelated process (the number was reused).
#[cfg(any(test, unix))]
fn started_after(elapsed_secs: u64, now_ms: u64, marker_started_ms: u64) -> bool {
    // `ps` shows whole seconds, so allow a little slack.
    now_ms.saturating_sub(elapsed_secs.saturating_mul(1000)) > marker_started_ms.saturating_add(2000)
}

/// `ps` elapsed time (`[[dd-]hh:]mm:ss`) in seconds.
#[cfg(any(test, unix))]
fn parse_etime(text: &str) -> Option<u64> {
    let text = text.trim();
    let (days, rest) = match text.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, text),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut secs = 0u64;
    for p in parts {
        secs = secs * 60 + p.parse::<u64>().ok()?;
    }
    Some(days * 86_400 + secs)
}

/// Whether the process that wrote `marker` is still running: its pid exists and is not a
/// newer process that was given the same number.
pub fn marker_alive(marker: &Marker) -> bool {
    if !process_alive(marker.pid) {
        return false;
    }
    #[cfg(unix)]
    {
        let out = std::process::Command::new("ps").args(["-o", "etime=", "-p", &marker.pid.to_string()]).output();
        if let Some(elapsed) = out.ok().and_then(|o| parse_etime(&String::from_utf8_lossy(&o.stdout))) {
            return !started_after(elapsed, crate::shared::infrastructure::jsonl_log::now_ms(), marker.started_ms);
        }
    }
    true
}

/// The interrupted turns of `project`: markers whose process is gone (by `alive`), removed
/// as they are read, newest first. Markers of other projects and of live processes stay.
pub fn take_interrupted(state_dir: &Path, project: &Path, alive: &dyn Fn(&Marker) -> bool) -> Vec<Marker> {
    let project = project.to_string_lossy();
    let mut found: Vec<Marker> = Vec::new();
    for entry in std::fs::read_dir(state_dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(PREFIX) || !name.ends_with(".json") {
            continue;
        }
        let Some(marker) = std::fs::read_to_string(entry.path()).ok().and_then(|t| serde_json::from_str::<Marker>(&t).ok()) else {
            continue;
        };
        if marker.project == project && !alive(&marker) {
            let _ = std::fs::remove_file(entry.path());
            found.push(marker);
        }
    }
    found.sort_by_key(|m| std::cmp::Reverse(m.started_ms));
    found
}

/// The line shown at startup for the newest interrupted turn, when there is one.
pub fn notice(markers: &[Marker]) -> Option<String> {
    let newest = markers.first()?;
    let more = if markers.len() > 1 { format!(" (and {} earlier)", markers.len() - 1) } else { String::new() };
    Some(format!(
        "  The last session ended in the middle of a turn{more}: \"{}\" ({}). Nothing was lost but its answer; ask again, or /resume the session.",
        newest.prompt,
        utc_stamp(newest.started_ms).0
    ))
}

/// Startup: what to tell the person about turns that did not finish in `project`.
pub fn recovery_notice(state_dir: &Path, project: &Path) -> Option<String> {
    notice(&take_interrupted(state_dir, project, &marker_alive))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jev-inflight-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(state: &Path, pid: u32, project: &str, started_ms: u64, prompt: &str) {
        let m = Marker { pid, started_ms, project: project.into(), prompt: prompt.into() };
        std::fs::write(marker_path(state, pid), serde_json::to_string(&m).unwrap()).unwrap();
    }

    #[test]
    fn a_turn_that_ends_leaves_nothing_and_one_that_does_not_is_found() {
        let state = dir("lifecycle");
        begin(&state, Path::new("/p"), "fix the parser\nsecond", 5);
        assert!(marker_path(&state, std::process::id()).exists());
        end(&state);
        assert!(!marker_path(&state, std::process::id()).exists());
        // Our own marker, left behind and with its process "gone".
        begin(&state, Path::new("/p"), "fix the parser\nsecond", 5);
        let found = take_interrupted(&state, Path::new("/p"), &|_: &Marker| false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].prompt, "fix the parser");
        assert!(take_interrupted(&state, Path::new("/p"), &|_: &Marker| false).is_empty(), "a marker is reported once");
    }

    #[test]
    fn live_processes_and_other_projects_are_left_alone() {
        let state = dir("others");
        write(&state, 111, "/p", 1, "live one");
        write(&state, 222, "/other", 2, "other project");
        write(&state, 333, "/p", 3, "dead one");
        let found = take_interrupted(&state, Path::new("/p"), &|m: &Marker| m.pid == 111);
        assert_eq!(found.iter().map(|m| m.prompt.as_str()).collect::<Vec<_>>(), ["dead one"]);
        assert!(marker_path(&state, 111).exists() && marker_path(&state, 222).exists());
        std::fs::write(state.join("inflight-9.json"), "garbage").unwrap();
        let rest = take_interrupted(&state, Path::new("/p"), &|_: &Marker| false);
        assert_eq!(rest.iter().map(|m| m.prompt.as_str()).collect::<Vec<_>>(), ["live one"], "garbage is skipped");
    }

    #[test]
    fn the_notice_names_the_newest_and_counts_the_rest() {
        assert_eq!(notice(&[]), None);
        let m = |t, p: &str| Marker { pid: 1, started_ms: t, project: "/p".into(), prompt: p.into() };
        let text = notice(&[m(2000, "newest"), m(1000, "older")]).unwrap();
        assert!(text.contains("\"newest\"") && text.contains("(and 1 earlier)") && text.contains("1970-01-01"), "{text}");
    }

    #[test]
    fn a_reused_pid_is_not_the_process_that_wrote_the_marker() {
        assert_eq!(parse_etime("05:07"), Some(307));
        assert_eq!(parse_etime("1-02:03:04"), Some(86_400 + 7_384));
        assert_eq!(parse_etime("junk"), None);
        // A marker written at t=10s; a process that has run 5s at t=1000s began long after it.
        assert!(started_after(5, 1_000_000, 10_000));
        assert!(!started_after(995, 1_000_000, 10_000));
        #[cfg(unix)]
        {
            let now = crate::shared::infrastructure::jsonl_log::now_ms();
            let own = |started_ms| Marker { pid: std::process::id(), started_ms, project: "/p".into(), prompt: String::new() };
            assert!(marker_alive(&own(now)), "a marker this process wrote is alive");
            assert!(!marker_alive(&own(0)), "a marker from long before this process started is not");
        }
    }

    #[test]
    fn this_process_is_alive_and_an_impossible_pid_is_not() {
        assert!(process_alive(std::process::id()));
        #[cfg(unix)]
        assert!(!process_alive(0x7fff_fff0));
    }
}
