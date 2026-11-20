//! `/race` (the same prompt to two providers, side by side, keep the winner) and
//! `/ask-other` (a second opinion from a different provider). Rendering and decisions only;
//! the engine runs the turns.

use crate::tui::render::{display_width, truncate_to_width, wrap_to_width, Style};

/// One racer's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Racer {
    pub provider: String,
    pub text: String,
    pub cost_usd: f64,
    pub ok: bool,
}

/// How a race ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Keep racer `0` or `1`: the other one failed.
    OnlyOneAnswered(usize),
    /// The answers match, so keep the cheaper racer.
    SameAnswerCheaper(usize),
    /// Both answered differently: the person picks.
    YouPick,
    /// Neither answered.
    BothFailed,
}

fn normalised(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn outcome(a: &Racer, b: &Racer) -> Outcome {
    match (a.ok, b.ok) {
        (false, false) => Outcome::BothFailed,
        (true, false) => Outcome::OnlyOneAnswered(0),
        (false, true) => Outcome::OnlyOneAnswered(1),
        (true, true) if normalised(&a.text) == normalised(&b.text) => Outcome::SameAnswerCheaper(if b.cost_usd < a.cost_usd { 1 } else { 0 }),
        (true, true) => Outcome::YouPick,
    }
}

/// Two columns of `width` columns in total, each headed by its provider, separated by `│`.
/// A column's text is wrapped, and the shorter one is padded with blank rows.
pub fn side_by_side(left: (&str, &str), right: (&str, &str), width: usize, style: Style) -> String {
    let col = (width.max(21) - 3) / 2;
    let layout = |(name, text): (&str, &str)| {
        let mut rows = vec![style.bold(&truncate_to_width(name, col))];
        rows.extend(text.trim().lines().flat_map(|l| wrap_to_width(l, col)));
        rows
    };
    let (l, r) = (layout(left), layout(right));
    (0..l.len().max(r.len()))
        .map(|i| {
            let lr = l.get(i).map_or("", String::as_str);
            let rr = r.get(i).map_or("", String::as_str);
            let pad = " ".repeat(col.saturating_sub(display_width(lr)));
            format!("{lr}{pad} {} {rr}", style.dim("│")).trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What is being checked by `/ask-other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Answer,
    Diff,
}

/// The prompt that asks a different model to review `subject_text`.
pub fn second_opinion_prompt(subject: Subject, subject_text: &str) -> String {
    let (what, ask) = match subject {
        Subject::Answer => ("answer", "Is it correct and complete? Point out mistakes, missing cases and anything you would do differently."),
        Subject::Diff => ("diff", "Review it for bugs, missed edge cases and risky changes."),
    };
    format!("Another assistant produced this {what}. {ask} Be concise; say \"looks right\" if it is.\n\n---\n{}\n---", subject_text.trim())
}

/// The provider for a second opinion: the first ready one that did not write the thing.
pub fn other_provider<'a>(author: &str, ready: &'a [String]) -> Option<&'a str> {
    ready.iter().map(String::as_str).find(|p| *p != author)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::strip_ansi;

    fn r(p: &str, text: &str, cost: f64, ok: bool) -> Racer {
        Racer { provider: p.into(), text: text.into(), cost_usd: cost, ok }
    }

    #[test]
    fn outcomes() {
        assert_eq!(outcome(&r("a", "x", 1.0, true), &r("b", "x", 0.5, false)), Outcome::OnlyOneAnswered(0));
        assert_eq!(outcome(&r("a", "x", 1.0, false), &r("b", "x", 0.5, true)), Outcome::OnlyOneAnswered(1));
        assert_eq!(outcome(&r("a", "x", 1.0, false), &r("b", "x", 0.5, false)), Outcome::BothFailed);
        assert_eq!(outcome(&r("a", "The  answer\nis 4", 1.0, true), &r("b", "the answer is 4", 0.5, true)), Outcome::SameAnswerCheaper(1));
        assert_eq!(outcome(&r("a", "4", 0.1, true), &r("b", "4", 0.5, true)), Outcome::SameAnswerCheaper(0));
        assert_eq!(outcome(&r("a", "4", 0.1, true), &r("b", "5", 0.5, true)), Outcome::YouPick);
    }

    #[test]
    fn columns_line_up_and_pad() {
        let out = side_by_side(("claude", "one two three four"), ("codex", "short"), 41, Style::off());
        let rows: Vec<&str> = out.lines().collect();
        assert_eq!(rows[0], format!("claude{} │ codex", " ".repeat(13)));
        assert_eq!(rows[1], format!("one two three four{} │ short", " "));
        for row in &rows {
            assert!(display_width(row) <= 41, "{row:?}");
        }
        let long = side_by_side(("a", &"word ".repeat(30)), ("b", "x"), 41, Style::off());
        assert!(long.lines().count() > 3);
        assert!(long.lines().all(|l| display_width(l) <= 41));
        assert_eq!(strip_ansi(&side_by_side(("a", "x"), ("b", "y"), 41, Style::on())), side_by_side(("a", "x"), ("b", "y"), 41, Style::off()));
    }

    #[test]
    fn second_opinion() {
        let p = second_opinion_prompt(Subject::Diff, "  +a\n-b ");
        assert!(p.contains("this diff") && p.ends_with("---\n+a\n-b\n---"));
        assert!(second_opinion_prompt(Subject::Answer, "x").contains("this answer"));
        let ready = vec!["claude".to_string(), "codex".to_string()];
        assert_eq!(other_provider("claude", &ready), Some("codex"));
        assert_eq!(other_provider("grok", &ready), Some("claude"));
        assert_eq!(other_provider("claude", &ready[..1]), None);
    }
}
