//! A server question uses its own form, never the permission mode's approval card.

use super::{layout::raised_row, theme::Theme, approval::choice, CARD_INPUT_ROWS};
use crate::app::screen_state::Elicitation;
use crate::render::wrap_to_width;
use jev_approval::domain::visible_text::visible;
use jev_backend::{ElicitationKind, FieldKind};

pub(super) fn card(e: &Elicitation, width: usize, th: &Theme) -> (Vec<String>, Option<(usize, usize)>) {
    let inner = width.saturating_sub(4).max(2);
    let mut rows = vec![raised_row(&th.strong(&format!("{} needs your input", visible(&e.request.server))), width, th)];
    let mut body = Vec::new();
    for line in e.request.message.split('\n') { body.extend(wrap_to_width(&visible(line), inner)); }
    match &e.request.kind {
        ElicitationKind::Url(url) => {
            body.extend(wrap_to_width(&format!("Open this URL in your browser: {}", visible(url)), inner));
            body.extend(wrap_to_width("Choose Send answer to acknowledge the request. Complete the action in your browser.", inner));
        }
        ElicitationKind::Unsupported(reason) => body.extend(wrap_to_width(&visible(reason), inner)),
        _ => {},
    }
    if !e.editing() {
        for (field, value) in e.fields().iter().zip(&e.values) {
            body.extend(wrap_to_width(&format!("{}: {}", visible(&field.title), visible(value)), inner));
        }
    }
    let first = e.scroll.min(body.len().saturating_sub(CARD_INPUT_ROWS));
    for line in body.iter().skip(first).take(CARD_INPUT_ROWS) { rows.push(raised_row(line, width, th)); }
    if body.len() > CARD_INPUT_ROWS {
        rows.push(raised_row(&th.muted("PgUp/PgDn to read the rest"), width, th));
    }
    let mut cursor = None;
    rows.push(raised_row("", width, th));
    if e.editing() {
        let field = &e.fields()[e.field];
        let required = if field.required { "required" } else { "optional; leave blank to omit" };
        for line in wrap_to_width(&format!("{} ({}/{}, {required})", visible(&field.title), e.field + 1, e.fields().len()), inner) {
            rows.push(raised_row(&th.strong(&line), width, th));
        }
        for line in wrap_to_width(&visible(&field.description), inner) { rows.push(raised_row(&line, width, th)); }
        let hint = match &field.kind {
            FieldKind::Boolean => "true / false · ↑↓ to choose".into(),
            FieldKind::Choice(choices) => format!("{} · ↑↓ to choose", choices.iter().map(|c| visible(c)).collect::<Vec<_>>().join(" / ")),
            FieldKind::Number => "Enter a number".into(), FieldKind::Integer => "Enter a whole number".into(),
            FieldKind::Text => String::new(),
        };
        for line in wrap_to_width(&hint, inner) { rows.push(raised_row(&th.muted(&line), width, th)); }
        let (text, at) = e.editor.layout(inner.saturating_sub(1));
        let start = at.0.saturating_sub(2);
        cursor = Some((rows.len() + at.0 - start, at.1 + 2));
        for line in text.iter().skip(start).take(3) { rows.push(raised_row(&visible(line), width, th)); }
        if let Some(error) = &e.error { rows.push(raised_row(&th.warning(error), width, th)); }
        rows.push(th.keys(&[("Enter", "next"), ("Shift+Tab", "back"), ("Esc", "cancel")], width));
    } else {
        for (i, label) in e.choices().iter().enumerate() { rows.push(raised_row(&choice(i, label, i == e.selected, th), width, th)); }
        rows.push(th.keys(&[("↑↓", "move"), ("Enter", "confirm"), ("Shift+Tab", "edit"), ("Esc", "cancel")], width));
    }
    (rows, cursor)
}
