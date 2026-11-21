//! The one-line route announcement, and the short tag made from the engine's long
//! reason.

use crate::render::style::{Style, Theme};

pub use jev_turn::domain::route_facts::RouteFacts;

struct ParsedReason {
    task: Option<String>,
    why: String,
}

/// `explain (0.91)` → `explain`; `explain (0.31)` → `explain (unsure)`.
fn task_label(head: &str) -> String {
    let t = head.split(" → ").next().unwrap_or(head).trim();
    let Some((name, paren)) = t.split_once(" (") else { return t.to_string() };
    let paren = paren.trim_end_matches(')').trim();
    let tag = match paren.parse::<f64>() {
        Ok(v) if v < 0.5 => " (unsure)",
        Ok(_) => "",
        Err(_) if paren == "security rule" => " (rule)",
        Err(_) if paren.eq_ignore_ascii_case("jev unavailable") => " (no Jev)",
        Err(_) => "",
    };
    format!("{}{tag}", name.trim())
}

/// Drop `(0.91)`-style confidences from `s`, except a low one (< 0.5), which becomes
/// `(unsure)`. Other parentheses are kept.
fn soften_confidence(s: &str) -> String {
    let (mut out, mut rest) = (String::new(), s);
    while let Some(i) = rest.find('(') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find(')') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let inner = &after[..j];
        match inner.trim().parse::<f64>() {
            Ok(v) if inner.contains('.') => {
                if v < 0.5 {
                    out.push_str("(unsure)");
                } else {
                    out.truncate(out.trim_end().len());
                }
            }
            _ => {
                out.push('(');
                out.push_str(inner);
                out.push(')');
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// The word after `prefix` in `c` (`switched from claude (…)` → `claude`).
fn word_after<'a>(c: &'a str, prefix: &str) -> &'a str {
    let rest = c.strip_prefix(prefix).unwrap_or("");
    rest.split([' ', '(']).next().unwrap_or("")
}

/// One `; `-separated piece of the engine's reason in a few words.
fn short_clause(clause: &str) -> String {
    let c = clause.trim();
    if c == "first turn" {
        "first turn".into()
    } else if c.starts_with("pinned provider") || c.starts_with("pinned model") {
        "pinned".into()
    } else if c.starts_with("same task, warm session") {
        "warm session".into()
    } else if c.starts_with("switched from ") {
        format!("switched from {}", word_after(c, "switched from "))
    } else if c.starts_with("not confident enough to switch from ") {
        format!("unsure, staying on {}", word_after(c, "not confident enough to switch from "))
    } else if c.starts_with("holding ") {
        "holding for cache".into()
    } else if c.starts_with("stepped down") {
        "stepped down".into()
    } else if c.starts_with("upgrade within") {
        "upgraded".into()
    } else if let Some(rest) = c.strip_prefix("steered off ") {
        let pct = rest.split("plan ").nth(1).and_then(|p| p.split('%').next()).unwrap_or("");
        let who = rest.split([' ', '(']).next().unwrap_or("");
        if pct.is_empty() {
            format!("steered off {who}")
        } else {
            format!("steered off {who} (plan {pct}%)")
        }
    } else if c.starts_with("Jev unavailable") {
        "no Jev, keeping model".into()
    } else if c.contains("cannot serve this turn any more") {
        "model unavailable".into()
    } else if c.starts_with("orchestrating:") {
        "orchestrating".into()
    } else if c.starts_with("orchestration skipped") {
        "orchestration skipped".into()
    } else if let Some(i) = c.find(" (pin ").filter(|_| c.contains("ignored: routing is off")) {
        let before = soften_confidence(&c[..i]);
        if before.is_empty() {
            "routing off, pin ignored".into()
        } else {
            format!("{before} · routing off, pin ignored")
        }
    } else {
        soften_confidence(c)
    }
}

fn parse_reason(reason: &str) -> ParsedReason {
    let r = reason.trim();
    let (task, tail) = if let Some(rest) = r.strip_prefix("task:") {
        let rest = rest.trim_start();
        let (head, tail) = rest.split_once("; ").unwrap_or((rest, ""));
        (Some(task_label(head)), tail)
    } else if let Some((_, why)) = r.split_once(" — ") {
        (None, why)
    } else {
        (None, r)
    };
    let why = tail.split("; ").map(short_clause).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" · ");
    ParsedReason { task, why }
}

/// The engine's long route reason as a short tag: `task: explain (0.91) → claude
/// sonnet-5-5; first turn` becomes `explain · first turn`. Confidence decimals are
/// dropped unless low (below 0.5), which reads `(unsure)`.
pub fn short_reason(reason: &str) -> String {
    let p = parse_reason(reason);
    [p.task.unwrap_or_default(), p.why].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// The one-line route announcement: `→ codex · gpt-6-luna   simple_edit · pinned`.
pub fn route_line(f: &RouteFacts, style: Style) -> String {
    let th = Theme::new(style);
    let p = parse_reason(&f.reason);
    let mut tag: Vec<String> = Vec::new();
    match p.task {
        Some(t) => tag.push(t),
        None if !f.task.is_empty() => tag.push(f.task.clone()),
        None => {}
    }
    if !p.why.is_empty() {
        tag.push(p.why);
    }
    if f.switched_provider && !tag.iter().any(|t| t.contains("switched")) {
        tag.push("switched provider".into());
    }
    if f.dropped_tier {
        tag.push("lower tier".into());
    }
    let who = if f.model.is_empty() {
        th.name(&f.provider)
    } else {
        format!("{} {} {}", th.name(&f.provider), style.dim("·"), f.model)
    };
    let head = format!("{} {who}", style.cyan("→"));
    if tag.is_empty() {
        head
    } else {
        format!("{head}   {}", style.dim(&tag.join(" · ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::strip_ansi;
    use crate::render::test_facts::*;

    #[test]
    fn short_reasons() {
        for (long, short) in [
            ("task: explain (0.91) → claude sonnet-5-5; first turn", "explain · first turn"),
            ("task: simple_edit (0.88) → codex gpt-6-luna; pinned provider codex", "simple_edit · pinned"),
            ("task: simple_edit (0.88) → codex gpt-6.1-sol; pinned model", "simple_edit · pinned"),
            ("task: explain (0.31) → claude sonnet-5-5; first turn", "explain (unsure) · first turn"),
            ("task: explain (0.50) → claude sonnet-5-5; first turn", "explain · first turn"),
            ("task: security (security rule) → claude opus-5-5; first turn", "security (rule) · first turn"),
            ("task: unclear (Jev unavailable) → claude sonnet-5-5; Jev unavailable, keeping the current model", "unclear (no Jev) · no Jev, keeping model"),
            ("task: feature (0.93) → codex gpt-6.1-sol; switched from claude (new task, confident)", "feature · switched from claude"),
            ("task: review (0.70) → claude opus-5-5; not confident enough to switch from claude (new task, confident)", "review · unsure, staying on claude"),
            ("task: docs (0.90) → claude sonnet-5-5; same task, warm session (same provider)", "docs · warm session"),
            ("task: docs (0.90) → claude sonnet-5-5; holding claude-opus-5-5 for the cache, easier turn 1/2", "docs · holding for cache"),
            ("task: docs (0.90) → claude sonnet-5-5; stepped down after 2 easier turns", "docs · stepped down"),
            ("task: feature (0.90) → claude opus-5-5; upgrade within claude", "feature · upgraded"),
            ("task: feature (0.90) → codex gpt-6.1-sol; switched from claude (new task, confident); steered off claude (plan 91% used)", "feature · switched from claude · steered off claude (plan 91%)"),
            ("task: feature (0.90) → codex x; first turn; orchestrating: codex x plans, Jev picks the workers", "feature · first turn · orchestrating"),
            ("task: feature (0.90) → codex x; first turn; orchestration skipped: no delegate binary", "feature · first turn · orchestration skipped"),
            ("task: feature (0.90) → codex x; claude-opus-5-5 cannot serve this turn any more", "feature · model unavailable"),
            ("claude sonnet-5-5 — tier 1 (pin codex ignored: routing is off, claude only)", "tier 1 · routing off, pin ignored"),
            ("claude sonnet-5-5 — everyday (0.77)", "everyday"),
            ("", ""),
            ("something else entirely", "something else entirely"),
        ] {
            assert_eq!(short_reason(long), short, "{long}");
        }
    }

    #[test]
    fn route_lines() {
        let facts = |provider: &str, model: &str, reason: &str| RouteFacts {
            provider: provider.into(),
            model: model.into(),
            reason: reason.into(),
            ..Default::default()
        };
        assert_eq!(
            route_line(&facts("codex", "gpt-6-luna", "task: simple_edit (0.88) → codex gpt-6-luna; pinned provider codex"), OFF),
            "→ codex · gpt-6-luna   simple_edit · pinned"
        );
        assert_eq!(
            route_line(&facts("claude", "sonnet-5-5", "task: explain (0.91) → claude sonnet-5-5; first turn"), OFF),
            "→ claude · sonnet-5-5   explain · first turn"
        );
        // no reason at all: just the model
        assert_eq!(route_line(&facts("grok", "grok-5", ""), OFF), "→ grok · grok-5");
        // the task field is used when the reason has none
        let mut f = facts("codex", "gpt", "everyday");
        f.task = "explain".into();
        f.switched_provider = true;
        f.dropped_tier = true;
        assert_eq!(route_line(&f, OFF), "→ codex · gpt   explain · everyday · switched provider · lower tier");
        // a reason that already says "switched" is not repeated
        let mut f = facts("codex", "gpt", "task: feature (0.9) → codex gpt; switched from claude (x)");
        f.switched_provider = true;
        assert_eq!(route_line(&f, OFF), "→ codex · gpt   feature · switched from claude");
        // provider only
        assert_eq!(route_line(&facts("codex", "", ""), OFF), "→ codex");
    }

    #[test]
    fn route_line_colours_the_provider_and_dims_the_reason() {
        let f = RouteFacts {
            provider: "codex".into(),
            model: "gpt".into(),
            reason: "task: explain (0.9) → codex gpt; first turn".into(),
            ..Default::default()
        };
        let s = route_line(&f, ON);
        assert!(s.contains("\x1b[32mcodex\x1b[0m"));
        assert!(s.contains("\x1b[2mexplain · first turn\x1b[0m"));
        assert_eq!(strip_ansi(&s), route_line(&f, OFF));
    }
}
