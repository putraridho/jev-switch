//! `/workflows`: what orchestration is doing in this project: whether it is on, the workers
//! still open (running, or done and waiting for the planner to merge or discard them), the
//! recent delegations with who did them and how they went, and the last day's totals.

use super::style::{Style, Theme};
use super::text::human_duration;
use super::width::{clamp_width, truncate_to_width};

/// An open worker, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWorkerRow {
    pub id: String,
    pub provider: String,
    pub model: String,
    /// How long ago it started.
    pub age_ms: u64,
    pub running: bool,
    pub shown: usize,
    pub merged: usize,
}

/// One delegation attempt, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationRow {
    /// `2h ago`.
    pub ago: String,
    pub id: String,
    pub task: String,
    pub provider: String,
    pub model: String,
    pub ok: bool,
    pub error_kind: Option<String>,
    pub wall_ms: u64,
    pub files: usize,
    pub tools: usize,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowFacts {
    pub orchestrate: bool,
    pub open: Vec<OpenWorkerRow>,
    pub recent: Vec<DelegationRow>,
    /// Over the last 24 hours: delegations, how many succeeded, worker time.
    pub day: (usize, usize, u64),
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub fn workflows_text(f: &WorkflowFacts, style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let th = Theme::new(style);
    let who = |p: &str, m: &str| format!("{} {} {m}", th.name(p), style.dim("·"));
    let mut out = Vec::new();
    let state = if f.orchestrate {
        format!("{} {}", style.green("on"), style.dim("· a planner may hand independent pieces to cheaper workers · /orchestrate off"))
    } else {
        format!("{} {}", style.yellow("off"), style.dim("· every turn runs on one model · /orchestrate on"))
    };
    out.push(format!("{} {state}", style.bold("Orchestration")));
    out.push(String::new());

    out.push(style.bold("Open workers"));
    if f.open.is_empty() {
        out.push(format!("  {}", style.dim("none: nothing is running or waiting for review")));
    }
    for w in &f.open {
        let status = if w.running {
            style.cyan(&format!("running {}", human_duration(w.age_ms)))
        } else {
            let seen = style.dim(&format!(" · {} seen, {} merged", plural(w.shown, "file", "files"), w.merged));
            format!("{}{seen}", style.yellow("waiting for the planner to review"))
        };
        out.push(truncate_to_width(&format!("  {}  {}   {status}", style.bold(&w.id), who(&w.provider, &w.model)), width));
    }
    out.push(String::new());

    out.push(style.bold("Recent delegations"));
    if f.recent.is_empty() {
        out.push(format!("  {}", style.dim("none in this project yet; with orchestration on, a big task is split across workers")));
    }
    let task_w = f.recent.iter().map(|r| r.task.chars().count()).max().unwrap_or(0);
    for r in f.recent.iter().rev() {
        let outcome = if r.ok {
            let mut s = format!("{} {}", style.green("✓"), human_duration(r.wall_ms));
            s.push_str(&style.dim(&format!(" · {} · {}", plural(r.files, "file", "files"), plural(r.tools, "tool call", "tool calls"))));
            s
        } else {
            let why = r.error_kind.clone().unwrap_or_else(|| "failed".into());
            let retry = if r.attempt > 1 { format!(" (attempt {})", r.attempt) } else { String::new() };
            format!("{} {}", style.red("✗"), style.red(&format!("{why}{retry}")))
        };
        let line = format!("  {}  {}  {:<task_w$}  {}   {outcome}", style.dim(&format!("{:>8}", r.ago)), r.id, r.task, who(&r.provider, &r.model));
        out.push(truncate_to_width(&line, width));
    }
    let (n, ok, ms) = f.day;
    if n > 0 {
        out.push(String::new());
        out.push(style.dim(&format!("Last 24 hours: {} · {ok} done · {} failed · {} of worker time", plural(n, "delegation", "delegations"), n - ok, human_duration(ms))));
    }
    out.iter().map(|l| truncate_to_width(l, width)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::testutil::*;
    use crate::uifmt::{display_width, strip_ansi};

    fn facts() -> WorkflowFacts {
        WorkflowFacts {
            orchestrate: true,
            open: vec![
                OpenWorkerRow { id: "w1".into(), provider: "codex".into(), model: "gpt-6.1-sol".into(), age_ms: 130_000, running: true, shown: 0, merged: 0 },
                OpenWorkerRow { id: "w2".into(), provider: "gemini".into(), model: "gemini-4-argon".into(), age_ms: 9_000, running: false, shown: 2, merged: 0 },
            ],
            recent: vec![
                DelegationRow { ago: "2h ago".into(), id: "w0".into(), task: "tests".into(), provider: "codex".into(), model: "gpt-6".into(), ok: false, error_kind: Some("quota".into()), wall_ms: 4000, files: 0, tools: 0, attempt: 1 },
                DelegationRow { ago: "5m ago".into(), id: "w1".into(), task: "feature".into(), provider: "claude".into(), model: "claude-sonnet-5-5".into(), ok: true, error_kind: None, wall_ms: 72_000, files: 2, tools: 18, attempt: 2 },
            ],
            day: (2, 1, 76_000),
        }
    }

    #[test]
    fn workflows_show_the_state_open_workers_recent_delegations_and_the_day() {
        let s = workflows_text(&facts(), OFF, 120);
        assert!(s.starts_with("Orchestration on · a planner may hand independent pieces"), "{s}");
        assert!(s.contains("  w1  codex · gpt-6.1-sol   running 2m10s"), "{s}");
        assert!(s.contains("  w2  gemini · gemini-4-argon   waiting for the planner to review · 2 files seen, 0 merged"), "{s}");
        let newest = s.find("w1  feature").unwrap_or(usize::MAX);
        let older = s.find("w0  tests").unwrap_or(0);
        assert!(newest < older, "newest first: {s}");
        assert!(s.contains("claude · claude-sonnet-5-5   ✓ 1m12s · 2 files · 18 tool calls"), "{s}");
        assert!(s.contains("✗ quota"), "{s}");
        assert!(s.ends_with("Last 24 hours: 2 delegations · 1 done · 1 failed · 1m16s of worker time"), "{s}");
        assert_eq!(strip_ansi(&workflows_text(&facts(), ON, 120)), s);
        for line in workflows_text(&facts(), OFF, 40).lines() {
            assert!(display_width(line) <= 40, "{line}");
        }
    }

    #[test]
    fn an_empty_project_says_what_orchestration_would_do() {
        let s = workflows_text(&WorkflowFacts::default(), OFF, 100);
        assert!(s.starts_with("Orchestration off · every turn runs on one model · /orchestrate on"), "{s}");
        assert!(s.contains("none: nothing is running or waiting for review"));
        assert!(s.contains("none in this project yet"));
        no_esc(&s);
    }
}
