//! Antigravity's quota for the Gemini provider, from `agy -p /usage --output-format json`.
//! The call spends no tokens. `agy`'s background updater is switched off for it, so asking
//! never starts a download.

use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use tokio::process::Command;

use crate::PlanUsage;

const DEADLINE: Duration = Duration::from_secs(45);
/// How close a reset must be to "now plus the whole window" to count as a placeholder.
const BLANK_TOLERANCE_S: u64 = 120;
/// The Gemini buckets: id, the window name Claude and Codex readings use, and its length.
/// The `3p-*` buckets (other vendors' models inside Antigravity) are not this provider's.
const WINDOWS: [(&str, &str, u64); 2] = [("gemini-5h", "five_hour", 5 * 3600), ("gemini-weekly", "seven_day", 7 * 86_400)];

#[derive(Deserialize)]
struct Output {
    status: Option<String>,
    command: Option<CommandOutput>,
}
#[derive(Deserialize)]
struct CommandOutput {
    data: Option<Data>,
}
#[derive(Deserialize)]
struct Data {
    groups: Vec<Group>,
}
#[derive(Deserialize)]
struct Group {
    buckets: Vec<Bucket>,
}
#[derive(Deserialize)]
struct Bucket {
    id: String,
    remaining_fraction: f64,
    reset_time: Option<String>,
}

/// The busiest Gemini window of one `/usage` answer read at `now_s`.
///
/// `None` for any other shape, and for the blank answer `agy` gives when it is signed out
/// (and has been seen to give while signed in): every window full with its reset exactly one
/// window ahead. Showing that as "0% used" would be a guess.
pub fn plan_from_usage(output: &str, now_s: u64) -> Option<PlanUsage> {
    let output: Output = serde_json::from_str(output).ok()?;
    if output.status.as_deref() != Some("SUCCESS") {
        return None;
    }
    let buckets: Vec<Bucket> = output.command?.data?.groups.into_iter().flat_map(|g| g.buckets).collect();
    let mut evidence = false;
    let mut busiest: Option<PlanUsage> = None;
    for (id, window, length) in WINDOWS {
        let Some(bucket) = buckets.iter().find(|b| b.id == id && (0.0..=1.0).contains(&b.remaining_fraction)) else { continue };
        let resets_at = bucket.reset_time.as_deref().and_then(parse_utc).filter(|at| *at > 0);
        let untouched = bucket.remaining_fraction >= 1.0 && resets_at.is_none_or(|at| at.abs_diff(now_s.saturating_add(length)) <= BLANK_TOLERANCE_S);
        evidence |= !untouched;
        let plan = PlanUsage { window: window.to_owned(), utilization: 1.0 - bucket.remaining_fraction, resets_at };
        if busiest.as_ref().is_none_or(|b| plan.utilization > b.utilization) {
            busiest = Some(plan);
        }
    }
    busiest.filter(|_| evidence)
}

/// Ask `agy` (the binary at `bin`) for its usage. `None` on any failure.
pub async fn fetch(bin: &str) -> Option<PlanUsage> {
    let mut command = Command::new(bin);
    command
        .args(["-p", "/usage", "--output-format", "json"])
        // Not the project folder: asking for usage must not touch workspace trust.
        .current_dir(std::env::temp_dir())
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(DEADLINE, command.output()).await.ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    let now_s = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    plan_from_usage(std::str::from_utf8(&output.stdout).ok()?, now_s)
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z` as seconds since the epoch.
fn parse_utc(text: &str) -> Option<u64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let time = time.split_once('.').map_or(time, |(whole, _)| whole);
    let numbers = |s: &str, sep: char| -> Option<[u64; 3]> {
        let parts: Vec<u64> = s.split(sep).map(|p| p.parse().ok().filter(|_| p.len() <= 4)).collect::<Option<_>>()?;
        parts.try_into().ok()
    };
    let [year, month, day] = numbers(date, '-')?;
    let [hour, minute, second] = numbers(time, ':')?;
    if year < 1970 || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = (y % 400) * 365 + (y % 400) / 4 - (y % 400) / 100 + day_of_year;
    let days = ((y / 400) * 146_097 + day_of_era).checked_sub(719_468)?;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-26T04:40:00Z.
    const NOW: u64 = 1_795_668_000;

    fn answer(buckets: &[(&str, f64, &str)]) -> String {
        let list: Vec<String> = buckets.iter().map(|(id, left, reset)| format!(r#"{{"id":"{id}","name":"n","remaining_fraction":{left},"reset_time":"{reset}"}}"#)).collect();
        format!(r#"{{"status":"SUCCESS","command":{{"data":{{"groups":[{{"buckets":[{}]}}]}}}}}}"#, list.join(","))
    }

    #[test]
    fn the_busiest_gemini_window_wins_and_other_vendors_are_ignored() {
        let output = answer(&[("gemini-weekly", 0.75, "2026-12-01T05:44:11Z"), ("gemini-5h", 0.5, "2026-11-26T07:05:23.5Z"), ("3p-5h", 0.0, "2026-11-26T07:05:23Z")]);
        let plan = plan_from_usage(&output, NOW).unwrap();
        assert_eq!(plan, PlanUsage { window: "five_hour".into(), utilization: 0.5, resets_at: Some(1_795_676_723) });
    }

    #[test]
    fn the_blank_answer_is_no_reading() {
        let output = answer(&[("gemini-weekly", 1.0, "2026-12-03T04:40:20Z"), ("gemini-5h", 1.0, "2026-11-26T09:40:00Z")]);
        assert_eq!(plan_from_usage(&output, NOW), None);
    }

    #[test]
    fn other_shapes_are_no_reading() {
        for bad in ["", "not json", r#"{"status":"ERROR"}"#, r#"{"status":"SUCCESS"}"#] {
            assert_eq!(plan_from_usage(bad, NOW), None, "{bad}");
        }
        assert_eq!(plan_from_usage(&answer(&[("gemini-5h", 1.5, "2026-11-26T07:05:23Z")]), NOW), None);
        let no_reset = plan_from_usage(&answer(&[("gemini-5h", 0.5, "soon")]), NOW).unwrap();
        assert_eq!(no_reset.resets_at, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fetch_asks_with_the_updater_switched_off() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile_dir();
        let stub = dir.join("agy");
        let body = answer(&[("gemini-5h", 0.25, "2099-01-01T00:00:00Z")]);
        std::fs::write(&stub, format!("#!/bin/sh\n[ \"$*\" = \"-p /usage --output-format json\" ] && [ \"$AGY_CLI_DISABLE_AUTO_UPDATE\" = true ] || exit 1\necho '{body}'\n")).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plan = fetch(stub.to_str().unwrap()).await.unwrap();
        assert_eq!(plan.utilization, 0.75);
        assert_eq!(fetch("/nonexistent/agy").await, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-agy-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
