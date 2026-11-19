//! Providers as the user sees them: the shared row type, the startup glyph line and
//! the `/providers` table.

use super::style::{Style, Theme};
use super::text::one_line;
use super::width::{clamp_width, display_width, pad_to_width, truncate_to_width};

/// Whether a provider can be used, mirroring the availability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus {
    Ready,
    NotInstalled,
    NotLoggedIn,
    NoKey,
    Unknown,
}

impl ProviderStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInstalled => "not installed",
            Self::NotLoggedIn => "not logged in",
            Self::NoKey => "no key",
            Self::Unknown => "unknown",
        }
    }
}

/// One provider as `/providers` and the no-route message show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailRow {
    pub provider: String,
    pub status: ProviderStatus,
    pub plan: Option<String>,
    /// Models the login lists; `None` means every model the router knows.
    pub models: Option<Vec<String>>,
    /// Why, in a few words, from the probe.
    pub detail: String,
    pub verified: bool,
    /// How long a usage-limit cooldown has left, e.g. `2h 10m`.
    pub cooldown: Option<String>,
    /// The CLI that runs it when the ID does not say (`gemini` runs `agy` without Gemini CLI).
    pub via: Option<String>,
}

impl AvailRow {
    pub fn new(provider: &str, status: ProviderStatus) -> Self {
        Self {
            provider: provider.into(),
            status,
            plan: None,
            models: None,
            detail: String::new(),
            verified: true,
            cooldown: None,
            via: None,
        }
    }
}

fn status_glyph(status: ProviderStatus, cooling: bool, style: Style) -> String {
    match (status, cooling) {
        (ProviderStatus::Ready, false) => style.green("●"),
        (ProviderStatus::Ready, true) => style.yellow("◐"),
        (ProviderStatus::NotLoggedIn | ProviderStatus::NoKey, _) => style.yellow("○"),
        (ProviderStatus::NotInstalled, _) => style.dim("✕"),
        (ProviderStatus::Unknown, _) => style.dim("?"),
    }
}

/// The providers on one line, each as its status glyph and name: `● claude  ○ codex  ✕ gemini`
/// (the same glyphs as [`providers_table`]), cut to `width`. For the startup banner.
pub fn provider_glyphs(rows: &[AvailRow], style: Style, width: usize) -> String {
    let th = Theme::new(style);
    let line = rows
        .iter()
        .map(|r| format!("{} {}", status_glyph(r.status, r.cooldown.is_some(), style), th.provider(&r.provider, &r.provider)))
        .collect::<Vec<_>>()
        .join("  ");
    truncate_to_width(&line, clamp_width(width))
}

/// `/providers`: one aligned row per provider: glyph (● ready, ○ not logged in, ✕ not
/// installed, ? unknown, ◐ ready but cooling down), name, plan, then the models (or why
/// it is not usable) cut to fit `width`, `(unverified)` when the probe was never checked
/// against the real CLI.
pub fn providers_table(rows: &[AvailRow], style: Style, width: usize) -> String {
    let width = clamp_width(width);
    let th = Theme::new(style);
    let name_w = rows.iter().map(|r| display_width(&r.provider)).max().unwrap_or(0);
    let plans: Vec<String> = rows.iter().map(|r| r.plan.as_deref().map_or("-".to_string(), one_line)).collect();
    let plan_w = plans.iter().map(|p| display_width(p)).max().unwrap_or(1);
    let mut lines = Vec::new();
    for (r, plan) in rows.iter().zip(&plans) {
        let cooling = r.cooldown.is_some();
        let prefix = format!(
            "  {} {} {} ",
            status_glyph(r.status, cooling, style),
            th.provider(&r.provider, &pad_to_width(&r.provider, name_w)),
            style.dim(&pad_to_width(plan, plan_w)),
        );
        let main_plain = if r.status == ProviderStatus::Ready {
            r.models.as_ref().map_or("all models".to_string(), |m| one_line(&m.join(", ")))
        } else {
            let detail = one_line(&r.detail);
            if detail.is_empty() || detail == r.status.label() {
                r.status.label().to_string()
            } else {
                format!("{} — {detail}", r.status.label())
            }
        };
        let mut suffix_plain = String::new();
        if let Some(v) = r.via.as_deref().filter(|v| *v != r.provider) {
            suffix_plain.push_str(&format!(" · via {v}"));
        }
        if !r.verified {
            suffix_plain.push_str(" (unverified)");
        }
        if let Some(left) = &r.cooldown {
            suffix_plain.push_str(&format!(" · cooling {}", one_line(left)));
        }
        let room = width.saturating_sub(display_width(&prefix)).max(1);
        let suffix_w = display_width(&suffix_plain);
        let (main_fit, show_suffix) = if display_width(&main_plain) + suffix_w <= room {
            (main_plain, true)
        } else if room >= suffix_w + 6 {
            (truncate_to_width(&main_plain, room - suffix_w), true)
        } else {
            (truncate_to_width(&main_plain, room), false)
        };
        let main = match r.status {
            ProviderStatus::Ready => main_fit,
            ProviderStatus::NotLoggedIn | ProviderStatus::NoKey => style.yellow(&main_fit),
            _ => style.dim(&main_fit),
        };
        let mut line = format!("{prefix}{main}");
        if show_suffix {
            if !r.verified {
                line.push_str(&style.dim(" (unverified)"));
            }
            if let Some(left) = &r.cooldown {
                line.push_str(&style.yellow(&format!(" · cooling {}", one_line(left))));
            }
        }
        lines.push(truncate_to_width(&line, width));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uifmt::{strip_ansi, MIN_WIDTH};
    use crate::uifmt::testutil::*;

    #[test]
    fn providers_table_layout() {
        let s = providers_table(&rows(), OFF, 100);
        assert_eq!(
            s,
            "  ◐ claude   max_5x all models · cooling 2h 10m\n  ○ codex    -      not logged in — run codex login\n  ✕ gemini   -      not installed\n  ○ deepseek -      no key\n  ? grok     -      unknown (unverified)"
        );
        assert_eq!(providers_table(&[], OFF, 80), "");
    }

    #[test]
    fn providers_table_glyphs_and_models() {
        let mut ready = AvailRow::new("codex", ProviderStatus::Ready);
        ready.plan = Some("plus".into());
        ready.models = Some(vec!["gpt-6.1-sol".into(), "gpt-6-luna".into()]);
        let s = providers_table(&[ready.clone()], OFF, 80);
        assert_eq!(s, "  ● codex plus gpt-6.1-sol, gpt-6-luna");
        let mut unverified = ready;
        unverified.verified = false;
        assert!(providers_table(&[unverified], OFF, 80).ends_with("gpt-6-luna (unverified)"));
        let styled = providers_table(&rows(), ON, 100);
        assert!(styled.contains("\x1b[33m◐\x1b[0m"));
        assert!(styled.contains("\x1b[33m○\x1b[0m"));
        assert!(styled.contains("\x1b[2m✕\x1b[0m"));
        assert!(styled.contains("\x1b[2m?\x1b[0m"));
        assert!(styled.contains("\x1b[2m (unverified)\x1b[0m"));
        assert_eq!(strip_ansi(&styled), providers_table(&rows(), OFF, 100));
        let all = AvailRow::new("claude", ProviderStatus::Ready);
        assert!(providers_table(&[all], ON, 80).contains("\x1b[32m●\x1b[0m"));
    }

    #[test]
    fn providers_table_fits_every_width() {
        let mut long = AvailRow::new("codex", ProviderStatus::Ready);
        long.plan = Some("pro".into());
        long.models = Some((0..30).map(|i| format!("model-number-{i}")).collect());
        long.verified = false;
        long.cooldown = Some("3h 20m".into());
        let mut all = rows();
        all.push(long);
        for w in [0, 10, MIN_WIDTH, 25, 30, 40, 60, 80, 120] {
            for style in [ON, OFF] {
                for l in providers_table(&all, style, w).split('\n') {
                    assert!(display_width(l) <= clamp_width(w), "{l:?} at {w}");
                }
            }
        }
        let s = providers_table(&all[5..], OFF, 60);
        assert!(s.contains("…") && s.ends_with("(unverified) · cooling 3h 20m"), "{s}");
    }

    #[test]
    fn provider_glyphs_fit_on_one_line_and_use_the_table_glyphs() {
        let line = provider_glyphs(&rows(), OFF, 80);
        assert_eq!(line, "◐ claude  ○ codex  ✕ gemini  ○ deepseek  ? grok");
        assert!(display_width(&provider_glyphs(&rows(), OFF, 20)) <= 20);
        assert_eq!(strip_ansi(&provider_glyphs(&rows(), ON, 80)), line);
        assert!(provider_glyphs(&rows(), ON, 80).contains("\x1b["), "coloured when styled");
    }
}
