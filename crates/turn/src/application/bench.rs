//! `/bench`: which model has actually served you best. The ranking is built from the audit
//! log (see [`jev_shared::domain::audit`]): per model, how many turns answered, how long they took and how
//! many tokens they used, so the router's guesses can be checked against your own work.
//!
//! Besides answers, it shows accepted work: how many of the turns whose work was checked held
//! up, how many had to be redone, and what an accepted turn cost, always with the number of
//! turns behind it (`n=`). A rate over fewer than [`MIN_VERIFIED`] checked turns is shown but
//! marked as too few to rely on, and routing does not use it.
//!
//! The task sets that `/bench` is meant to run across providers on demand (`eval/cases.json`
//! or your own `.jev/bench.json`) are listed here by [`count_tasks`], but running them spends
//! provider quota and is not done by this module.

use std::path::Path;

use serde::Deserialize;

use jev_shared::domain::audit::{Entry, Event, Outcome};
use crate::domain::outcomes::{Outcomes, Tally};
use jev_routing::domain::cache_facts::CacheBook;
use jev_routing::domain::track_record::{Rate, MIN_VERIFIED};
use jev_shared::domain::wording::human_duration;

/// One model's record.
#[derive(Debug, Clone, PartialEq)]
pub struct Rank {
    pub model: String,
    pub turns: u64,
    pub answered: u64,
    pub median_ms: u64,
    pub avg_out_tokens: u64,
    /// How the turns whose work was checked came out (all task types together).
    pub work: Tally,
}

impl Rank {
    /// Share of turns that answered, in percent.
    pub fn success_pct(&self) -> u64 {
        self.answered * 100 / self.turns.max(1)
    }
}

/// The least turns a model needs to be ranked: one lucky turn is not a record.
pub const MIN_TURNS: u64 = 3;

fn median(sorted: &[u64]) -> u64 {
    match sorted.len() {
        0 => 0,
        n if n % 2 == 1 => sorted[n / 2],
        n => (sorted[n / 2 - 1] + sorted[n / 2]) / 2,
    }
}

/// Models with at least [`MIN_TURNS`] turns, best first: higher success, then faster median,
/// then name (so the order never depends on hash order).
pub fn rank(entries: &[Entry]) -> Vec<Rank> {
    #[derive(Default)]
    struct Acc {
        turns: u64,
        answered: u64,
        times: Vec<u64>,
        out: u64,
    }
    let mut by_model: std::collections::BTreeMap<&str, Acc> = std::collections::BTreeMap::new();
    for e in entries {
        let Event::Turn { model, outcome, elapsed_ms, out_tokens, .. } = &e.event else { continue };
        if model.is_empty() {
            continue;
        }
        let a = by_model.entry(model).or_default();
        a.turns += 1;
        a.out += out_tokens;
        if *outcome == Outcome::Answered {
            a.answered += 1;
            a.times.push(*elapsed_ms);
        }
    }
    let work = Outcomes::from_entries(entries).by_model().into_iter().map(|(m, t)| (m.to_string(), t)).collect::<std::collections::BTreeMap<_, _>>();
    let mut ranks: Vec<Rank> = by_model
        .into_iter()
        .filter(|(_, a)| a.turns >= MIN_TURNS)
        .map(|(model, mut a)| {
            a.times.sort_unstable();
            Rank { model: model.to_string(), turns: a.turns, answered: a.answered, median_ms: median(&a.times), avg_out_tokens: a.out / a.turns, work: work.get(model).cloned().unwrap_or_default() }
        })
        .collect();
    ranks.sort_by(|a, b| b.success_pct().cmp(&a.success_pct()).then(a.median_ms.cmp(&b.median_ms)).then(a.model.cmp(&b.model)));
    ranks
}

#[derive(Deserialize)]
struct AnyTask {}

/// How many tasks a task file (a JSON array of objects) holds; 0 when it is missing or not one.
pub fn count_tasks(path: &Path) -> usize {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Vec<AnyTask>>(&t).ok()).map_or(0, |v| v.len())
}

/// "accepted 3/4 (n=4) 75% [30-95%]  rework 1  ~12k tokens per accepted", or nothing when no
/// turn of the model was checked.
fn work_line(t: &Tally) -> Option<String> {
    let rate: Rate = t.accepted_rate()?;
    let cost = t.tokens_per_accepted().map_or(String::new(), |n| format!("  ~{} tokens per accepted", compact(n)));
    let few = if t.enough() { String::new() } else { format!("  (n<{MIN_VERIFIED}: too few to rely on)") };
    Some(format!(
        "accepted {}/{} (n={}) {:.0}% [{:.0}-{:.0}%]  rework {}{cost}{few}",
        t.accepted,
        t.verified(),
        t.verified(),
        rate.point * 100.0,
        rate.low * 100.0,
        rate.high * 100.0,
        t.rework
    ))
}

fn compact(n: u64) -> String {
    if n >= 10_000 { format!("{}k", n / 1000) } else { n.to_string() }
}

/// The checked work per model and task type, for `/bench`.
pub fn work_text(outcomes: &Outcomes) -> String {
    let rows: Vec<String> = outcomes.iter().filter_map(|(m, task, t)| work_line(t).map(|l| format!("  {m} · {task}: {l}"))).collect();
    if rows.is_empty() {
        return String::new();
    }
    format!("  Accepted work by task type (only turns whose work was checked; /verify turns checking on)\n{}", rows.join("\n"))
}

/// `/bench`: the ranking and the task sets found in `project`.
pub fn bench_text(ranks: &[Rank], project: &Path) -> String {
    let mut lines = Vec::new();
    if ranks.is_empty() {
        lines.push(format!("  No ranking yet: a model needs {MIN_TURNS} recorded turns (see /audit)."));
    } else {
        lines.push("  Your models, best first (from your own turns)".to_string());
        for (i, r) in ranks.iter().enumerate() {
            let tests = work_line(&r.work).map_or(String::new(), |l| format!("  {l}"));
            lines.push(format!(
                "  {}. {:<34} {:>3}% answered of {:<3} median {:>5.1}s  ~{} output tokens{tests}",
                i + 1,
                r.model,
                r.success_pct(),
                r.turns,
                r.median_ms as f64 / 1000.0,
                r.avg_out_tokens
            ));
        }
    }
    for (label, file) in [("eval/cases.json", "eval/cases.json"), (".jev/bench.json", ".jev/bench.json")] {
        let n = count_tasks(&project.join(file));
        if n > 0 {
            lines.push(format!("  {label}: {n} tasks to run across providers (not run from here: it spends quota)"));
        }
    }
    lines.join("\n")
}

/// The prompt-cache facts per (provider, model, backend), for `/bench`: the share of input read
/// from the cache with the number of turns behind it, and how long the cache is believed to
/// live, each marked `observed` (counted from turns that reported usage) or `assumed` (a
/// built-in prior). Empty until a turn has been counted.
pub fn cache_text(book: &CacheBook) -> String {
    let rows: Vec<String> = book
        .iter()
        .filter_map(|(key, facts)| {
            let hit = facts.hit_percent()?;
            let ttl = facts.ttl(key.provider);
            let backend = if key.backend.is_empty() {
                String::new()
            } else {
                format!(" [{}]", key.backend)
            };
            Some(
                format!(
                    "  {} {}{backend}: {hit}% of input read from cache (observed, n={}); lives ~{} ({})",
                    key.provider,
                    key.model,
                    facts.samples,
                    human_duration(ttl.ms),
                    ttl.basis.label()
                )
            )
        })
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "  Prompt cache by model (advisory: counted from turns that reported usage; a window nobody observed is assumed)\n{}",
        rows.join("\n")
    )
}

/// Everything `/bench` shows for the audit `entries` of the project in `project`, with what
/// the session's prompt caches showed.
pub fn bench_report(entries: &[Entry], project: &Path, cache: &CacheBook) -> String {
    let text = bench_text(&rank(entries), project);
    [text, work_text(&Outcomes::from_entries(entries)), cache_text(cache)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_shared::domain::acceptance::{Acceptance, Quality, Rejection, Rework};

    fn turn(model: &str, outcome: Outcome, ms: u64, out: u64) -> Entry {
        Entry { ts_ms: 0, project: "/p".into(), event: Event::Turn { model: model.into(), outcome, measurement: jev_shared::Measurement::Reported, elapsed_ms: ms, in_tokens: 1, out_tokens: out, prompt: String::new(), cached_tokens: 0, context_tokens: 0, tool_calls: 0, verdict: None, quality: Default::default() } }
    }

    #[test]
    fn ranks_by_success_then_speed_and_needs_enough_turns() {
        let mut entries = Vec::new();
        for ms in [3000, 1000, 2000] {
            entries.push(turn("a", Outcome::Answered, ms, 10));
        }
        for ms in [500, 600] {
            entries.push(turn("b", Outcome::Answered, ms, 10));
        }
        entries.push(turn("b", Outcome::Failed, 9, 10));
        for ms in [100, 100, 100] {
            entries.push(turn("c", Outcome::Answered, ms, 30));
        }
        entries.push(turn("lonely", Outcome::Answered, 1, 1));
        entries.push(turn("", Outcome::NoRoute, 0, 0));
        let ranks = rank(&entries);
        assert_eq!(ranks.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        assert_eq!((ranks[1].median_ms, ranks[1].success_pct(), ranks[1].avg_out_tokens), (2000, 100, 10));
        assert_eq!((ranks[2].success_pct(), ranks[2].median_ms), (66, 550));
    }

    #[test]
    fn the_text_says_when_there_is_nothing_and_counts_task_files() {
        let dir = std::env::temp_dir().join(format!("jev-bench-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("eval")).unwrap();
        assert!(bench_text(&[], &dir).contains("No ranking yet"));
        std::fs::write(dir.join("eval/cases.json"), r#"[{"request":"a"},{"request":"b"}]"#).unwrap();
        std::fs::create_dir_all(dir.join(".jev")).unwrap();
        std::fs::write(dir.join(".jev/bench.json"), "not json").unwrap();
        let text = bench_text(&[], &dir);
        assert!(text.contains("eval/cases.json: 2 tasks") && !text.contains(".jev/bench.json"), "{text}");
    }

    #[test]
    fn accepted_work_is_shown_with_its_sample_size_and_only_when_there_is_any() {
        let with = |acceptance, tokens, rework| {
            let mut e = turn("a", Outcome::Answered, 10, 1);
            if let Event::Turn { quality, .. } = &mut e.event {
                *quality = Quality { task: "feature".into(), acceptance, spent_tokens: tokens, rework, ..Quality::default() };
            }
            e
        };
        let bad = Acceptance::Rejected(Rejection::VerificationFailed);
        let entries = [with(Acceptance::Accepted, 1000, Rework::None), with(bad, 3000, Rework::None), with(Acceptance::Accepted, 2000, Rework::AfterFailedCheck), with(Acceptance::Unverified, 9, Rework::None)];
        let ranks = rank(&entries);
        assert_eq!((ranks[0].work.accepted, ranks[0].work.rejected, ranks[0].work.rework), (2, 1, 1));
        let text = bench_text(&ranks, Path::new("/nowhere"));
        assert!(text.contains("accepted 2/3 (n=3)") && text.contains("rework 1") && text.contains("~3000 tokens per accepted") && text.contains("too few to rely on"), "{text}");
        let by_task = work_text(&Outcomes::from_entries(&entries));
        assert!(by_task.contains("a · feature: accepted 2/3 (n=3)"), "{by_task}");
        let plain = rank(&[turn("b", Outcome::Answered, 1, 1), turn("b", Outcome::Answered, 1, 1), turn("b", Outcome::Answered, 1, 1)]);
        assert!(!bench_text(&plain, Path::new("/nowhere")).contains("accepted"));
        assert_eq!(work_text(&Outcomes::default()), "");
    }

    #[test]
    fn enough_checked_turns_drop_the_too_few_warning() {
        let t = Tally { turns: 12, accepted: 9, rejected: 3, ..Tally::default() };
        let line = work_line(&t).unwrap();
        assert!(line.contains("accepted 9/12 (n=12) 75%") && !line.contains("too few"), "{line}");
    }

    #[test]
    fn the_ranking_text_lists_each_model() {
        let r = Rank { model: "claude · s".into(), turns: 4, answered: 3, median_ms: 1500, avg_out_tokens: 42, work: Tally::default() };
        let text = bench_text(&[r], Path::new("/nowhere"));
        assert!(text.contains("1. claude · s") && text.contains(" 75% answered of 4") && text.contains("median   1.5s"), "{text}");
    }

    #[test]
    fn the_cache_section_shows_observed_ratio_with_n_and_assumed_or_observed_windows() {
        use jev_routing::domain::cache_facts::CacheKey;
        let mut book = CacheBook::default();
        assert!(cache_text(&book).is_empty(), "nothing counted: nothing claimed");
        let usage = |cached| jev_shared::Usage {
            input_tokens: 1_000,
            cached_input_tokens: cached,
            output_tokens: 1,
            ..Default::default()
};
        let claude = CacheKey::new(jev_shared::ProviderId::Claude, "sonnet", "Claude login");
        book.observe(&claude, &usage(900), None, 1);
        book.observe(&claude, &usage(700), Some(60_000), 2);
        let gemini = CacheKey::new(jev_shared::ProviderId::Gemini, "flash", "");
        book.observe(&gemini, &usage(800), Some(600_000), 3);
        let text = cache_text(&book);
        assert!(
            text.contains(
                "claude sonnet [Claude login]: 80% of input read from cache (observed, n=2); lives ~5m00s (assumed)"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "gemini flash: 80% of input read from cache (observed, n=1); lives ~10m00s (observed)"
            ),
            "{text}"
        );
        let report = bench_report(&[], Path::new("/nowhere"), &book);
        assert!(
            report.contains("No ranking yet") && report.contains("Prompt cache by model"),
            "{report}"
        );
    }
}
