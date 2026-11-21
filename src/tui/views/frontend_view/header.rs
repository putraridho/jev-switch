//! The header row and the router text on its right.

use super::layout::spread;
use super::theme::Theme;
use crate::tui::app::screen_state::App;
use crate::tui::render::display_width;

/// The header row: where you are on the left, `main ±3 ~/Playground/jev-switch “name”`, the
/// branch dim and the folder bright; what the router has on the right,
/// `⇄ auto · claude 32% · codex 71%  │  context 12%`.
pub(super) fn header_row(app: &App, w: usize, th: &Theme) -> String {
    let mut right = router_text(app, th);
    if let Some(pct) = app.context {
        let text = format!("context {pct}%");
        let shown = if pct >= 80 { th.warning(&format!("{text} · /compact")) } else { th.muted(&text) };
        right.push_str(&format!("{}{shown}", th.muted("  │  ")));
    }
    let branch = app.git.as_ref().map(|g| format!("{} ", th.muted(g))).unwrap_or_default();
    let name = app.session_name.as_ref().map(|n| th.muted(&format!(" \u{201c}{n}\u{201d}"))).unwrap_or_default();
    // The folder gives way first, from its front (`…/project`): the branch, the session's
    // name and the router's side are what a glance looks for.
    let room = w.saturating_sub(display_width(&right) + 3 + display_width(&branch) + display_width(&name));
    let place = if room >= 8 { crate::tui::views::status_facts::keep_tail(&app.place, room) } else { app.place.clone() };
    spread(&format!(" {branch}{place}{name}"), &right, w)
}

/// How turns are routed and what each subscription has left:
/// `⇄ auto · claude 32% · codex 71% · gemini cooling 1h20m · orchestrate on`. A plan window
/// at 80% or more is yellow (the router will move off it soon).
fn router_text(app: &App, th: &Theme) -> String {
    let names = Theme::provider_theme(th.0);
    let mut parts = vec![format!("{} {}", th.accent("⇄"), app.route)];
    for p in &app.providers {
        let name = names.name(&p.name);
        let via = p.via.as_deref().filter(|v| *v != p.name).map(|v| th.muted(&format!(" ({v})"))).unwrap_or_default();
        let chip = match (&p.cooling, p.used) {
            (Some(left), _) => format!("{name}{via} {}", th.muted(&format!("cooling {left}"))),
            (None, Some(u)) if u >= 80 => format!("{name}{via} {}", th.warning(&format!("{u}%"))),
            (None, Some(u)) => format!("{name}{via} {}", th.muted(&format!("{u}%"))),
            (None, None) => match &p.credit {
                Some(c) => format!("{name}{via} {}", th.muted(c)),
                None => format!("{name}{via}"),
            },
        };
        parts.push(chip);
    }
    if app.providers.is_empty() {
        parts.push(th.warning("no provider ready · /doctor"));
    }
    if app.orchestrate {
        parts.push(th.muted("orchestrate on"));
    }
    parts.join(&th.muted(" · "))
}
