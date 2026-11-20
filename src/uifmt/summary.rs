//! The one dim line after a turn.

use super::style::{Style, Theme};
use super::text::{human_count, human_duration, plural};

/// What a finished turn looked like, for the one-line summary.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SummaryFacts {
    pub ok: bool,
    pub cancelled: bool,
    pub provider: String,
    pub model: String,
    pub elapsed_ms: u64,
    pub in_tokens: u64,
    pub cached_tokens: u64,
    pub out_tokens: u64,
    /// (provider, fraction of its plan window used, 0..1)
    pub plan: Option<(String, f64)>,
    /// (files, lines added, lines removed)
    pub files_changed: Option<(usize, u32, u32)>,
}

/// The one dim line after a turn: `✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) ·
/// out 189 · claude plan 32% · 3 files +12 −4`; `■ stopped · 3s` when cancelled; `✗`
/// when it failed. A plan window at 80% or more is yellow.
pub fn turn_summary(f: &SummaryFacts, style: Style) -> String {
    let th = Theme::new(style);
    let time = human_duration(f.elapsed_ms);
    if f.cancelled {
        return format!("{} {}", style.yellow("■"), style.dim(&format!("stopped · {time}")));
    }
    let mark = if f.ok { style.green("✓") } else { style.red("✗") };
    let mut parts: Vec<String> = Vec::new();
    if f.provider.is_empty() {
        if !f.ok {
            parts.push("failed".into());
        }
    } else if f.model.is_empty() {
        parts.push(th.name(&f.provider));
    } else {
        parts.push(format!("{} · {}", th.name(&f.provider), f.model));
    }
    parts.push(time);
    if f.in_tokens > 0 || f.out_tokens > 0 {
        let cached = if f.cached_tokens > 0 { format!(" (cached {})", human_count(f.cached_tokens)) } else { String::new() };
        parts.push(format!("in {}{cached}", human_count(f.in_tokens)));
        parts.push(format!("out {}", human_count(f.out_tokens)));
    }
    if let Some((p, pct)) = plan_pct(f) {
        let text = plan_text(p, pct);
        parts.push(if pct >= 80 { style.yellow(&text) } else { text });
    }
    if let Some(files) = files_text(f) {
        parts.push(files);
    }
    // Keep the outcome and model legible; usage is supporting information.
    let mut parts = parts.into_iter();
    let first = parts.next().unwrap_or_default();
    let rest = parts.collect::<Vec<_>>();
    if rest.is_empty() {
        format!("{mark} {first}")
    } else {
        format!("{mark} {first} {} {}", style.dim("·"), style.dim(&rest.join(" · ")))
    }
}

/// The turn's closing row on the interactive screen, with only what a person looks for and
/// quiet so the next prompt's band stands out: `  Worked for 14s · ✓ codex · 3 files +12 −4`;
/// `  ■ Stopped after 3s` when cancelled. The model is on the route row that led the turn;
/// token counts live in `/cost`; the plan window shows only once it is 80% used or more.
pub fn turn_closing(f: &SummaryFacts, style: Style) -> String {
    let time = human_duration(f.elapsed_ms);
    if f.cancelled {
        return format!("  {} {}", style.yellow("■"), style.dim(&format!("Stopped after {time}")));
    }
    let mark = if f.ok { style.green("✓") } else { style.red("✗") };
    let mut parts = vec![style.dim(&format!("Worked for {time}"))];
    if !f.provider.is_empty() {
        parts.push(format!("{mark} {}", Theme::new(style).name(&f.provider)));
    } else if !f.ok {
        parts.push(format!("{mark} {}", style.dim("failed")));
    }
    if let Some((p, pct)) = plan_pct(f).filter(|(_, pct)| *pct >= 80) {
        parts.push(style.yellow(&plan_text(p, pct)));
    }
    if let Some(files) = files_text(f) {
        parts.push(style.dim(&files));
    }
    format!("  {}", parts.join(&style.dim(" · ")))
}

/// The plan window's provider and how full it is, in whole percent (at most 100).
fn plan_pct(f: &SummaryFacts) -> Option<(&str, u32)> {
    f.plan.as_ref().map(|(p, used)| (p.as_str(), (used.clamp(0.0, 1.0) * 100.0).round() as u32))
}

fn plan_text(provider: &str, pct: u32) -> String {
    format!("{provider} plan {pct}%")
}

/// `3 files +12 −4`, or `None` when no file changed.
fn files_text(f: &SummaryFacts) -> Option<String> {
    let (n, added, removed) = f.files_changed.filter(|(n, _, _)| *n > 0)?;
    let lines = if added == 0 && removed == 0 { String::new() } else { format!(" +{added} −{removed}") };
    Some(format!("{}{lines}", plural(n, "file", "files")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::strip_ansi;
    use crate::uifmt::testutil::*;

    #[test]
    fn turn_summaries() {
        assert_eq!(
            turn_summary(&facts(), OFF),
            "✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) · out 189 · claude plan 32%"
        );
        let mut f = facts();
        f.files_changed = Some((3, 12, 4));
        assert_eq!(
            turn_summary(&f, OFF),
            "✓ codex · gpt-6-luna · 14s · in 45K (cached 40K) · out 189 · claude plan 32% · 3 files +12 −4"
        );
        f.files_changed = Some((1, 0, 0));
        f.plan = None;
        f.cached_tokens = 0;
        assert_eq!(turn_summary(&f, OFF), "✓ codex · gpt-6-luna · 14s · in 45K · out 189 · 1 file");
        f.files_changed = Some((0, 0, 0));
        f.in_tokens = 0;
        f.out_tokens = 0;
        assert_eq!(turn_summary(&f, OFF), "✓ codex · gpt-6-luna · 14s");
        let cancelled = SummaryFacts { cancelled: true, elapsed_ms: 3_000, ..facts() };
        assert_eq!(turn_summary(&cancelled, OFF), "■ stopped · 3s");
        let failed = SummaryFacts { ok: false, elapsed_ms: 850, in_tokens: 0, out_tokens: 0, plan: None, ..facts() };
        assert_eq!(turn_summary(&failed, OFF), "✗ codex · gpt-6-luna · 850ms");
        let nothing = SummaryFacts { ok: false, elapsed_ms: 2_000, ..Default::default() };
        assert_eq!(turn_summary(&nothing, OFF), "✗ failed · 2s");
        let no_model = SummaryFacts { model: String::new(), ..facts() };
        assert!(turn_summary(&no_model, OFF).starts_with("✓ codex · 14s"));
    }

    #[test]
    fn the_closing_row_says_how_long_it_worked_and_who() {
        assert_eq!(turn_closing(&facts(), OFF), "  Worked for 14s · ✓ codex");
        let busy = SummaryFacts { plan: Some(("claude".into(), 0.91)), files_changed: Some((2, 5, 1)), ..facts() };
        assert_eq!(turn_closing(&busy, OFF), "  Worked for 14s · ✓ codex · claude plan 91% · 2 files +5 −1");
        let failed = SummaryFacts { ok: false, elapsed_ms: 850, ..facts() };
        assert_eq!(turn_closing(&failed, OFF), "  Worked for 850ms · ✗ codex");
        assert_eq!(turn_closing(&SummaryFacts { ok: false, elapsed_ms: 2_000, ..Default::default() }, OFF), "  Worked for 2s · ✗ failed");
        assert_eq!(turn_closing(&SummaryFacts { cancelled: true, elapsed_ms: 3_000, ..facts() }, OFF), "  ■ Stopped after 3s");
        let on = turn_closing(&facts(), ON);
        assert!(on.contains("\x1b[2mWorked for 14s") && on.contains("\x1b[32m✓"), "{on:?}");
    }

    #[test]
    fn turn_summary_colours() {
        let s = turn_summary(&facts(), ON);
        assert!(s.starts_with("\x1b[32m✓\x1b[0m "));
        assert!(s.contains("\x1b[2m"));
        assert_eq!(strip_ansi(&s), turn_summary(&facts(), OFF));
        assert!(s.contains("\x1b[32mcodex\x1b[0m"));
        let hot = SummaryFacts { plan: Some(("claude".into(), 0.93)), ..facts() };
        assert!(turn_summary(&hot, ON).contains("\x1b[33mclaude plan 93%\x1b[0m"));
        assert!(turn_summary(&hot, OFF).ends_with("claude plan 93%"));
        let over = SummaryFacts { plan: Some(("claude".into(), 7.0)), ..facts() };
        assert!(turn_summary(&over, OFF).ends_with("claude plan 100%"));
        let stopped = SummaryFacts { cancelled: true, ..facts() };
        assert!(turn_summary(&stopped, ON).starts_with("\x1b[33m■\x1b[0m"));
        let bad = SummaryFacts { ok: false, ..facts() };
        assert!(turn_summary(&bad, ON).starts_with("\x1b[31m✗\x1b[0m"));
    }
}
