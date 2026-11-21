//! The footer chip and the backend-native events, drawn in a provider's own identity.

use super::ProviderTheme;
use crate::render::gauge::{fuel_gauge, FuelLevel};
use crate::render::width::{clamp_width, truncate_to_width};
use crate::render::wrap::wrap_to_width;

/// Health of a provider as the footer chip shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    CoolingDown,
    Unavailable,
}

/// What a footer chip says about one provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ChipFacts {
    pub plan: Option<String>,
    /// Fraction of the plan window used, 0..1.
    pub used: Option<f64>,
    pub cooldown: Option<String>,
    pub health: Health,
}

/// A native event of a backend, shown faithfully in its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeEvent<'a> {
    Thinking(&'a str),
    Patch { files: &'a [String] },
    PlanStep { index: usize, total: usize, text: &'a str },
}

/// The lines of thinking or patch files shown before the rest is counted.
fn collapse_limit(provider: &str, event: &NativeEvent<'_>) -> usize {
    match (provider, event) {
        ("claude", NativeEvent::Thinking(_)) => 4,
        (_, NativeEvent::Thinking(_)) => 2,
        (_, NativeEvent::Patch { .. }) => 5,
        (_, NativeEvent::PlanStep { .. }) => 1,
    }
}

/// How a provider names its own events.
fn event_label(provider: &str, event: &NativeEvent<'_>) -> &'static str {
    match (provider, event) {
        ("codex", NativeEvent::Thinking(_)) => "reasoning",
        ("gemini", NativeEvent::Thinking(_)) => "thoughts",
        (_, NativeEvent::Thinking(_)) => "thinking",
        ("codex", NativeEvent::Patch { .. }) => "apply_patch",
        (_, NativeEvent::Patch { .. }) => "patch",
        (_, NativeEvent::PlanStep { .. }) => "plan",
    }
}

impl ProviderTheme {
    /// The header above an answer: who wrote it and with which model. A failover or race
    /// names every provider involved, in order.
    pub fn attribution_header(&self, authors: &[(&str, &str)]) -> String {
        let parts: Vec<String> = authors
            .iter()
            .map(|(p, m)| if m.is_empty() { self.chip(p) } else { format!("{} {} {m}", self.chip(p), self.style.dim("·")) })
            .collect();
        parts.join(&self.style.dim("  ⇄  "))
    }

    /// The answer set apart with a header and a thin gutter in the author's colour.
    /// `laid_out` must already fit `width - 2` columns.
    pub fn attributed_answer(&self, authors: &[(&str, &str)], laid_out: &str) -> String {
        let Some((first, _)) = authors.first() else { return format!("\n{}", laid_out.trim_end_matches('\n')) };
        let bar = self.paint(first, "▌");
        let mut rows = vec![String::new(), format!("{bar} {}", self.attribution_header(authors))];
        rows.extend(laid_out.trim_end_matches('\n').split('\n').map(|l| format!("{bar} {l}")));
        rows.join("\n")
    }

    /// A footer chip: `◆ claude max_5x ▰▰▰▱▱ 60% ⏳ 2h 10m`.
    pub fn footer_chip(&self, provider: &str, f: &ChipFacts) -> String {
        let mut parts = vec![self.chip(provider)];
        if let Some(plan) = &f.plan {
            parts.push(self.style.dim(plan));
        }
        if let Some(used) = f.used {
            let g = fuel_gauge(used, 5);
            let text = format!("{} {}%", g.bar, g.percent);
            parts.push(match g.level {
                FuelLevel::Ok => self.style.dim(&text),
                FuelLevel::Low => self.style.yellow(&text),
                FuelLevel::Critical => self.style.red(&text),
            });
        }
        match (f.health, &f.cooldown) {
            (Health::CoolingDown, Some(t)) => parts.push(self.style.yellow(&format!("⏳ {t}"))),
            (Health::CoolingDown, None) => parts.push(self.style.yellow("⏳")),
            (Health::Unavailable, _) => parts.push(self.style.red("✗ unavailable")),
            (Health::Ok, _) => {}
        }
        parts.join(" ")
    }

    /// A backend's own event under its own label, collapsed the way that backend's events
    /// are read (Claude's thinking longest, Codex patches as a file list, Gemini's plan
    /// steps numbered).
    pub fn native_event(&self, provider: &str, event: &NativeEvent<'_>, width: usize) -> String {
        let width = clamp_width(width);
        let label = event_label(provider, event);
        let head = format!("  {} {}", self.paint(provider, &self.identity(provider).glyph.to_string()), self.style.dim(label));
        let limit = collapse_limit(provider, event);
        let (detail, total): (Vec<String>, usize) = match event {
            NativeEvent::Thinking(text) => {
                let lines: Vec<String> = text.trim().lines().flat_map(|l| wrap_to_width(l, width.saturating_sub(6))).collect();
                let n = lines.len();
                (lines.into_iter().take(limit).map(|l| self.style.dim(&self.style.italic(&l))).collect(), n)
            }
            NativeEvent::Patch { files } => (files.iter().take(limit).map(|f| format!("{} {f}", self.style.green("+"))).collect(), files.len()),
            NativeEvent::PlanStep { index, total, text } => return truncate_to_width(&format!("{head} {}", self.style.dim(&format!("{index}/{total} {text}"))), width),
        };
        let mut rows = vec![head];
        rows.extend(detail.into_iter().map(|d| truncate_to_width(&format!("    {d}"), width)));
        if total > limit {
            rows.push(format!("    {}", self.style.dim(&format!("… {} more", total - limit))));
        }
        rows.join("\n")
    }
}
