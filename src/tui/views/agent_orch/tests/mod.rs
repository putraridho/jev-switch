use crate::tui::render::Style;

mod decision;
mod flow;
mod plan;
mod ribbon;
mod review;

pub(super) const OFF: Style = Style { enabled: false, terminal: false };
pub(super) const ON: Style = Style { enabled: true, terminal: true };
