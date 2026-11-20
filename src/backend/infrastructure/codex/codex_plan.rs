//! Read Codex's recorded plan limits without starting a provider process.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

use crate::backend::PlanUsage;

/// One rollout line: `{"type":"event_msg","payload":{"type":"token_count","rate_limits":{..}}}`.
/// Only the fields read here; anything else (and any other line type) is ignored.
#[derive(Deserialize)]
struct RolloutLine {
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    window_minutes: u64,
    resets_at: Option<u64>,
}

impl Window {
    /// A window the CLI reported sensibly; a nonsense percentage is dropped, not clamped.
    fn usage(&self) -> Option<PlanUsage> {
        if !(0.0..=100.0).contains(&self.used_percent) {
            return None;
        }
        Some(PlanUsage {
            window: match self.window_minutes {
                300 => "five_hour".to_owned(),
                10080 => "seven_day".to_owned(),
                n => format!("{n}m"),
            },
            utilization: self.used_percent / 100.0,
            resets_at: self.resets_at,
        })
    }
}

/// The busiest window (highest utilization) of a `token_count` line, as
/// `claude::plan_of` picks Claude's. Invalid or missing windows do not hide a usable one.
pub fn plan_from_line(line: &str) -> Option<PlanUsage> {
    let limits = serde_json::from_str::<RolloutLine>(line).ok()?.payload?.rate_limits?;
    [limits.primary, limits.secondary]
        .iter()
        .flatten()
        .filter_map(Window::usage)
        .max_by(|a, b| a.utilization.total_cmp(&b.utilization))
}

/// Read at most a few of the most recently modified rollouts. A recent session
/// may contain no usage yet, so older candidates can still supply a useful plan.
/// The file modification time records when that plan was observed locally.
pub fn latest_plan_in(sessions_dir: &Path) -> Option<(PlanUsage, SystemTime)> {
    fn collect(dir: &Path, depth: usize, files: &mut Vec<(SystemTime, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if depth > 0 && kind.is_dir() {
                collect(&entry.path(), depth - 1, files);
            } else if depth == 0 && kind.is_file() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                    if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                        files.push((modified, entry.path()));
                    }
                }
            }
        }
    }

    let mut files = Vec::new();
    collect(sessions_dir, 3, &mut files);
    files.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (modified, path) in files.into_iter().take(5) {
        let Ok(file) = File::open(path) else { continue };
        let mut last = None;
        for line in BufReader::new(file).lines() {
            match line {
                Ok(line) => {
                    if let Some(plan) = plan_from_line(&line) {
                        last = Some(plan);
                    }
                }
                Err(_) => break,
            }
        }
        if let Some(plan) = last {
            return Some((plan, modified));
        }
    }
    None
}

/// Resolve Codex's session directory from its configured home or the user's
/// home, matching the CLI's default without invoking it.
pub fn latest_plan() -> Option<(PlanUsage, SystemTime)> {
    latest_plan_in(&codex_home()?.join("sessions"))
}

/// `$CODEX_HOME`, else `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").filter(|s| !s.is_empty()).map(|h| PathBuf::from(h).join(".codex")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const SAMPLE: &str = r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":15.0,"window_minutes":300,"resets_at":1795124089},"secondary":{"used_percent":21.0,"window_minutes":10080,"resets_at":1795508228}}}}"#;

    fn temp_dir() -> PathBuf {
        crate::testkit::scratch_dir::unique_scratch_dir("codex-plan", "t")
    }

    #[test]
    fn parses_busiest_window() {
        let plan = plan_from_line(SAMPLE).unwrap();
        assert_eq!(plan.window, "seven_day");
        assert_eq!(plan.utilization, 0.21);
        assert_eq!(plan.resets_at, Some(1795508228));
    }

    #[test]
    fn handles_null_and_bad_lines() {
        let line = r#"{"payload":{"rate_limits":{"primary":null,"secondary":{"used_percent":12,"window_minutes":42}}}}"#;
        let plan = plan_from_line(line).unwrap();
        assert_eq!(plan.window, "42m");
        assert_eq!(plan.resets_at, None);
        assert!(plan_from_line("not json").is_none());
        assert!(plan_from_line(r#"{"payload":{"rate_limits":{"primary":null}}}"#).is_none());
        assert!(plan_from_line(r#"{"payload":{"type":"agent_message"}}"#).is_none());
        let over = r#"{"payload":{"rate_limits":{"primary":{"used_percent":150,"window_minutes":300}}}}"#;
        assert!(plan_from_line(over).is_none());
    }

    #[test]
    fn newest_file_and_last_valid_line_win() {
        let root = temp_dir();
        let day = root.join("2026/11/20");
        fs::create_dir_all(&day).unwrap();
        let old = day.join("rollout-old.jsonl");
        let new = day.join("rollout-new.jsonl");
        fs::write(&old, SAMPLE).unwrap();
        let last = r#"{"payload":{"rate_limits":{"primary":{"used_percent":30,"window_minutes":300}}}}"#;
        fs::write(&new, format!("{SAMPLE}\n{{bad json\n{last}\ninvalid\n")).unwrap();
        let old_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let new_time = SystemTime::UNIX_EPOCH + Duration::from_secs(2000);
        File::open(&old).unwrap().set_modified(old_time).unwrap();
        File::open(&new).unwrap().set_modified(new_time).unwrap();
        let (plan, observed) = latest_plan_in(&root).unwrap();
        assert_eq!(plan.window, "five_hour");
        assert_eq!(plan.utilization, 0.3);
        assert_eq!(observed, new_time);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_directory_is_empty() {
        let root = temp_dir();
        assert!(latest_plan_in(&root.join("missing")).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
