//! The subagent visuals: panel, footer cue, nested transcript, conflicts, result card and
//! timeline.

use std::collections::BTreeMap;

use crate::tui::views::agent_render::{bar, clamp_width, clean, fit, plural, usd};
use crate::tui::render::{display_width, human_count, human_duration, truncate_to_width, Style, Theme};

/// Where a worker is. Ordered the way the panel sorts: what needs you first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkerState {
    WaitingApproval,
    Running,
    Queued,
    Failed,
    Cancelled,
    Done,
}

impl WorkerState {
    pub fn word(self) -> &'static str {
        match self {
            Self::WaitingApproval => "waiting",
            Self::Running => "running",
            Self::Queued => "queued",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Done => "done",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Self::WaitingApproval => "?",
            Self::Running => "▶",
            Self::Queued => "·",
            Self::Failed => "✗",
            Self::Cancelled => "−",
            Self::Done => "✓",
        }
    }

    fn live(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval | Self::Queued)
    }

    fn paint(self, s: Style, text: &str) -> String {
        match self {
            Self::Done => s.green(text),
            Self::Failed => s.red(text),
            Self::WaitingApproval => s.yellow(text),
            Self::Running => s.cyan(text),
            Self::Queued | Self::Cancelled => s.dim(text),
        }
    }
}

/// One worker as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub task: String,
    pub state: WorkerState,
    pub elapsed_ms: u64,
    pub tokens: u64,
    pub cost_usd: Option<f64>,
    pub files: Vec<String>,
    /// "editing src/parser.rs": what it is doing right now.
    pub last_action: Option<String>,
    /// 0 for a worker of the planner, 1 for one a worker spawned.
    pub depth: usize,
}

impl WorkerInfo {
    pub fn new(id: &str, provider: &str, model: &str, task: &str, state: WorkerState) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            model: model.into(),
            task: task.into(),
            state,
            elapsed_ms: 0,
            tokens: 0,
            cost_usd: None,
            files: vec![],
            last_action: None,
            depth: 0,
        }
    }
}

/// Below this width the panel is one short line per worker.
const NARROW: usize = 70;

fn sorted(workers: &[WorkerInfo]) -> Vec<&WorkerInfo> {
    let mut v: Vec<&WorkerInfo> = workers.iter().collect();
    v.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| a.id.cmp(&b.id)));
    v
}

/// The `/agents` panel: every worker, needing-you first.
pub fn agents_panel(workers: &[WorkerInfo], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    if workers.is_empty() {
        return format!("  {}", style.dim("No workers yet. Workers appear here when a planner hands pieces off (/orchestrate on)."));
    }
    let theme = Theme::new(style);
    let done = workers.iter().filter(|w| w.state == WorkerState::Done).count();
    let mut out = vec![format!("  {}  {}", style.bold("Agents"), style.dim(&format!("{} {done} of {}", bar(done, workers.len(), 8), workers.len())))];
    for w in sorted(workers) {
        out.push(if width < NARROW { narrow_row(w, width, style, &theme) } else { wide_row(w, width, style, &theme) });
    }
    out.join("\n")
}

fn narrow_row(w: &WorkerInfo, width: usize, s: Style, theme: &Theme) -> String {
    let lead = format!("  {} {} {} ", w.state.glyph(), w.id, w.provider);
    let room = width.saturating_sub(display_width(&lead));
    format!("  {} {} {} {}", w.state.paint(s, w.state.glyph()), w.id, theme.provider(&w.provider, &w.provider), s.dim(&fit(&w.task, room)))
}

fn wide_row(w: &WorkerInfo, width: usize, s: Style, theme: &Theme) -> String {
    let indent = "  ".repeat(w.depth + 1);
    let state = format!("{} {:<9}", w.state.glyph(), w.state.word());
    let mut parts = vec![human_duration(w.elapsed_ms)];
    if w.tokens > 0 {
        parts.push(format!("{} tok", human_count(w.tokens)));
    }
    if let Some(c) = w.cost_usd {
        parts.push(usd(c));
    }
    if !w.files.is_empty() {
        parts.push(plural(w.files.len(), "file", "files"));
    }
    let meta = parts.join(" · ");
    let who = format!("{} {}", w.provider, w.model);
    let fixed = display_width(&indent) + display_width(&state) + 1 + display_width(&w.id) + 2 + display_width(&who) + 2 + display_width(&meta) + 2;
    let detail = w.last_action.as_deref().filter(|_| w.state.live()).unwrap_or(&w.task);
    let detail = fit(detail, width.saturating_sub(fixed));
    format!("{indent}{} {}  {}  {}  {}", w.state.paint(s, &state), w.id, theme.provider(&w.provider, &who), s.dim(&meta), detail)
}

/// The quiet footer cue: `4 workers · 2 running`; nothing when no worker exists.
pub fn workers_footer(workers: &[WorkerInfo]) -> Option<String> {
    if workers.is_empty() {
        return None;
    }
    let count = |st: WorkerState| workers.iter().filter(|w| w.state == st).count();
    let mut s = plural(workers.len(), "worker", "workers");
    for (st, label) in [(WorkerState::Running, "running"), (WorkerState::WaitingApproval, "waiting"), (WorkerState::Failed, "failed")] {
        if count(st) > 0 {
            s.push_str(&format!(" · {} {label}", count(st)));
        }
    }
    Some(s)
}

/// A worker's recorded steps, for the nested transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Activity {
    pub worker: WorkerInfo,
    /// One line per tool call, oldest first.
    pub steps: Vec<String>,
    /// The workers this one spawned.
    pub children: Vec<Activity>,
}

/// The `⇉ Delegate` row with its workers' activity under a tree gutter: one summary line
/// per worker when collapsed, the full tool-call stream when `expanded`.
pub fn nested_transcript(task: &str, group: &[Activity], expanded: bool, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let mut out = vec![format!("  ⇉ Delegate {}", fit(task, width.saturating_sub(14)))];
    for (i, a) in group.iter().enumerate() {
        tree_node(a, "    ", i + 1 == group.len(), expanded, width, style, &mut out);
    }
    out.join("\n")
}

fn tree_node(a: &Activity, prefix: &str, last: bool, expanded: bool, width: usize, s: Style, out: &mut Vec<String>) {
    let (branch, trunk) = if last { ("└─", "   ") } else { ("├─", "│  ") };
    let w = &a.worker;
    let summary = format!("{} {} {} {} · {} · {}", w.state.glyph(), w.id, w.provider, w.state.word(), plural(a.steps.len(), "step", "steps"), human_duration(w.elapsed_ms));
    out.push(format!("{prefix}{}{}", s.dim(branch), w.state.paint(s, &truncate_to_width(&summary, width.saturating_sub(display_width(prefix) + 2)))));
    let inner = format!("{prefix}{trunk}");
    if expanded {
        for step in &a.steps {
            out.push(format!("{inner}{}", s.dim(&fit(step, width.saturating_sub(display_width(&inner))))));
        }
    }
    for (i, c) in a.children.iter().enumerate() {
        tree_node(c, &inner, i + 1 == a.children.len(), expanded, width, s, out);
    }
}

/// Two or more workers touching one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    pub workers: Vec<String>,
}

/// The files more than one worker changed, in path order. Cancelled workers do not count.
pub fn file_conflicts(workers: &[WorkerInfo]) -> Vec<Conflict> {
    let mut by_file: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for w in workers.iter().filter(|w| w.state != WorkerState::Cancelled) {
        for f in &w.files {
            let ids = by_file.entry(f.as_str()).or_default();
            if !ids.contains(&w.id) {
                ids.push(w.id.clone());
            }
        }
    }
    by_file.into_iter().filter(|(_, ids)| ids.len() > 1).map(|(p, ids)| Conflict { path: p.into(), workers: ids }).collect()
}

/// A warning block for [`file_conflicts`]; `None` when the workers do not overlap.
pub fn conflict_warning(conflicts: &[Conflict], style: Style) -> Option<String> {
    if conflicts.is_empty() {
        return None;
    }
    let head = format!("! {} touched by more than one worker; merging the second will conflict:", plural(conflicts.len(), "file", "files"));
    let mut out = vec![format!("  {}", style.yellow(&head))];
    out.extend(conflicts.iter().map(|c| format!("    {} {}", clean(&c.path), style.dim(&format!("({})", c.workers.join(", "))))));
    Some(out.join("\n"))
}

/// What a finished worker leaves for the result card.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultFacts {
    pub worker: WorkerInfo,
    pub summary: String,
    pub tests: Option<String>,
    /// Why it failed, when it did.
    pub error: Option<String>,
}

/// The card shown when a worker finishes: outcome, files, tests, cost and the actions.
pub fn result_card(r: &ResultFacts, width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let w = &r.worker;
    let theme = Theme::new(style);
    let mut out = vec![format!(
        "  {} {}  {}  {}",
        w.state.paint(style, &format!("{} {}", w.state.glyph(), w.state.word())),
        w.id,
        theme.provider(&w.provider, &format!("{} {}", w.provider, w.model)),
        style.dim(&human_duration(w.elapsed_ms))
    )];
    let pad = "    ";
    let room = width.saturating_sub(pad.len());
    if !r.summary.is_empty() {
        out.push(format!("{pad}{}", fit(&r.summary, room)));
    }
    if let Some(e) = &r.error {
        out.push(format!("{pad}{}", style.red(&fit(&format!("reason: {e}"), room))));
    }
    match w.files.len() {
        0 => out.push(format!("{pad}{}", style.dim("files: none"))),
        n => out.push(format!("{pad}files ({n}): {}", fit(&w.files.join(", "), room.saturating_sub(12)))),
    }
    if let Some(t) = &r.tests {
        out.push(format!("{pad}tests: {}", fit(t, room.saturating_sub(7))));
    }
    let spend = [(w.tokens > 0).then(|| format!("{} tok", human_count(w.tokens))), w.cost_usd.map(usd)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    if !spend.is_empty() {
        out.push(format!("{pad}cost: {spend}"));
    }
    let actions = if w.state == WorkerState::Done {
        "[r] review  [a] accept  [d] discard".to_string()
    } else {
        format!("[t] retry  [d] discard   next: read the reason, then retry {} or do the piece yourself", w.id)
    };
    out.push(format!("{pad}{}", style.bold(&fit(&actions, room))));
    out.join("\n")
}

/// One worker's time on the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// The stretch inside the span spent waiting for an approval.
    pub waited: Option<(u64, u64)>,
}

/// Gantt lanes: `█` working, `░` waiting for approval, blank idle; the worker that ends
/// last is the critical path.
pub fn timeline(spans: &[Span], width: usize, style: Style) -> String {
    let width = clamp_width(width);
    let Some(total) = spans.iter().map(|s| s.end_ms).max().filter(|t| *t > 0) else {
        return format!("  {}", style.dim("No timeline yet: no worker has run."));
    };
    let label_w = spans.iter().map(|s| display_width(&s.id)).max().unwrap_or(0);
    let cells = width.saturating_sub(label_w + 24).clamp(8, 60);
    let cell_of = |ms: u64| ((ms as u128 * cells as u128) / total as u128) as usize;
    let mut out = vec![format!("  {}  {}", style.bold("Timeline"), style.dim(&format!("0 … {} (█ working  ░ waiting)", human_duration(total))))];
    for s in spans {
        let from = cell_of(s.start_ms).min(cells - 1);
        let to = cell_of(s.end_ms).clamp(from + 1, cells);
        let lane: String = (0..cells)
            .map(|c| {
                let waiting = s.waited.is_some_and(|(ws, we)| c >= cell_of(ws) && c < cell_of(we).max(cell_of(ws) + 1));
                match (c >= from && c < to, waiting) {
                    (false, _) => ' ',
                    (true, true) => '░',
                    (true, false) => '█',
                }
            })
            .collect();
        let tag = if s.end_ms == total { "  ◀ critical path" } else { "" };
        out.push(format!("  {:<label_w$} │{lane}│ {}{}", s.id, human_duration(s.end_ms.saturating_sub(s.start_ms)), style.yellow(tag)));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::strip_ansi;

    const OFF: Style = Style { enabled: false, terminal: false };
    const ON: Style = Style { enabled: true, terminal: true };

    fn w(id: &str, st: WorkerState) -> WorkerInfo {
        let mut x = WorkerInfo::new(id, "codex", "gpt-x", "write the parser tests", st);
        x.elapsed_ms = 14_000;
        x.tokens = 1200;
        x.cost_usd = Some(0.04);
        x
    }

    #[test]
    fn panel_sorts_what_needs_you_first_and_names_each_state_in_words() {
        let ws = [w("w1", WorkerState::Done), w("w2", WorkerState::WaitingApproval), w("w3", WorkerState::Running)];
        let p = agents_panel(&ws, 100, OFF);
        let rows: Vec<&str> = p.lines().collect();
        assert!(rows[0].contains("Agents") && rows[0].contains("1 of 3"), "{p}");
        assert!(rows[1].contains("waiting") && rows[1].contains("w2"));
        assert!(rows[2].contains("running"));
        assert!(rows[3].contains("done") && rows[3].contains("1.2K tok") && rows[3].contains("$0.04"));
        assert!(!p.contains('\x1b'));
    }

    #[test]
    fn a_running_worker_shows_its_last_action_instead_of_the_task() {
        let mut r = w("w1", WorkerState::Running);
        r.last_action = Some("editing src/parser.rs".into());
        assert!(agents_panel(&[r], 100, OFF).contains("editing src/parser.rs"));
    }

    #[test]
    fn narrow_terminals_get_one_short_line_per_worker() {
        let p = agents_panel(&[w("w1", WorkerState::Running), w("w2", WorkerState::Done)], 40, OFF);
        for l in p.lines() {
            assert!(display_width(l) <= 40, "{l:?}");
        }
        assert_eq!(p.lines().count(), 3);
    }

    #[test]
    fn long_text_never_overflows_the_width() {
        let mut x = w("w1", WorkerState::Running);
        x.task = "a very long task description that keeps going and going and going".into();
        for width in [40, 69, 70, 80, 120] {
            for l in agents_panel(&[x.clone()], width, OFF).lines() {
                assert!(display_width(l) <= width, "{width}: {l:?}");
            }
        }
    }

    #[test]
    fn styling_only_adds_escapes() {
        let ws = [w("w1", WorkerState::Failed), w("w2", WorkerState::Done)];
        assert_eq!(strip_ansi(&agents_panel(&ws, 100, ON)), agents_panel(&ws, 100, OFF));
        assert!(agents_panel(&ws, 100, ON).contains('\x1b'));
    }

    #[test]
    fn empty_panel_says_how_to_get_workers() {
        assert!(agents_panel(&[], 80, OFF).contains("/orchestrate on"));
    }

    #[test]
    fn footer_counts_workers_and_what_is_running() {
        assert_eq!(workers_footer(&[]), None);
        let ws = [w("a", WorkerState::Running), w("b", WorkerState::Running), w("c", WorkerState::Done), w("d", WorkerState::Queued)];
        assert_eq!(workers_footer(&ws).as_deref(), Some("4 workers · 2 running"));
        assert_eq!(workers_footer(&ws[..1]).as_deref(), Some("1 worker · 1 running"));
    }

    fn act(id: &str, steps: &[&str], children: Vec<Activity>) -> Activity {
        Activity { worker: w(id, WorkerState::Done), steps: steps.iter().map(|s| s.to_string()).collect(), children }
    }

    #[test]
    fn nested_transcript_collapses_to_one_line_per_worker_with_a_tree_gutter() {
        let g = [act("w1", &["Read a.rs", "Edit a.rs"], vec![]), act("w2", &["Bash cargo test"], vec![])];
        let t = nested_transcript("split the work", &g, false, 80, OFF);
        let rows: Vec<&str> = t.lines().collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("├─") && rows[1].contains("2 steps"));
        assert!(rows[2].contains("└─") && rows[2].contains("1 step "));
    }

    #[test]
    fn expanded_transcript_streams_tool_calls_and_nests_depth() {
        let g = [act("w1", &["Read a.rs"], vec![act("w1a", &["Grep x"], vec![])]), act("w2", &["Edit b.rs"], vec![])];
        let t = nested_transcript("t", &g, true, 80, OFF);
        let rows: Vec<&str> = t.lines().collect();
        assert!(rows.iter().any(|r| r.contains("│  Read a.rs")), "{t}");
        assert!(rows.iter().any(|r| r.contains("│  └─") && r.contains("w1a")), "{t}");
        assert!(rows.iter().any(|r| r.contains("   Edit b.rs") && !r.contains('│')), "last branch drops the trunk: {t}");
    }

    #[test]
    fn conflicts_name_files_two_workers_touch_and_ignore_cancelled_ones() {
        let mut a = w("w1", WorkerState::Done);
        a.files = vec!["src/a.rs".into(), "src/b.rs".into()];
        let mut b = w("w2", WorkerState::Running);
        b.files = vec!["src/b.rs".into()];
        let mut c = w("w3", WorkerState::Cancelled);
        c.files = vec!["src/a.rs".into()];
        let cs = file_conflicts(&[a.clone(), b, c]);
        assert_eq!(cs, vec![Conflict { path: "src/b.rs".into(), workers: vec!["w1".into(), "w2".into()] }]);
        let warn = conflict_warning(&cs, OFF).unwrap_or_default();
        assert!(warn.contains("src/b.rs") && warn.contains("w1, w2") && warn.contains("will conflict"));
        assert_eq!(conflict_warning(&file_conflicts(&[a]), OFF), None);
    }

    #[test]
    fn result_card_for_a_finished_worker_offers_review_accept_discard() {
        let mut x = w("w1", WorkerState::Done);
        x.files = vec!["a.rs".into(), "b.rs".into()];
        let c = result_card(&ResultFacts { worker: x, summary: "Added tests".into(), tests: Some("12 passed".into()), error: None }, 80, OFF);
        for want in ["done", "Added tests", "files (2): a.rs, b.rs", "tests: 12 passed", "$0.04", "[r] review", "[a] accept", "[d] discard"] {
            assert!(c.contains(want), "missing {want:?} in {c}");
        }
    }

    #[test]
    fn result_card_for_a_failure_gives_the_reason_and_next_step() {
        let c = result_card(&ResultFacts { worker: w("w1", WorkerState::Failed), summary: String::new(), tests: None, error: Some("limit hit".into()) }, 80, OFF);
        assert!(c.contains("reason: limit hit") && c.contains("[t] retry") && !c.contains("accept"), "{c}");
    }

    #[test]
    fn result_card_cleans_hostile_worker_text() {
        let c = result_card(&ResultFacts { worker: w("w1", WorkerState::Done), summary: "ok\x1b[2J\x1b]0;x\x07 done".into(), tests: None, error: None }, 80, OFF);
        assert!(!c.contains('\x1b') && !c.contains('\x07'));
    }

    #[test]
    fn timeline_shows_overlap_waits_and_the_critical_path() {
        let spans = [
            Span { id: "w1".into(), start_ms: 0, end_ms: 10_000, waited: None },
            Span { id: "w2".into(), start_ms: 2_000, end_ms: 20_000, waited: Some((10_000, 14_000)) },
        ];
        let t = timeline(&spans, 60, OFF);
        let rows: Vec<&str> = t.lines().collect();
        assert!(rows[0].contains("20s"));
        assert!(rows[1].contains('█') && !rows[1].contains("critical"));
        assert!(rows[2].contains('░') && rows[2].contains("critical path"));
        assert!(timeline(&[], 60, OFF).contains("No timeline"));
    }
}
