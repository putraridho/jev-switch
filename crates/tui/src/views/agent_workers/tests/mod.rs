use super::*;
use crate::render::Style;

mod conflicts;
mod panel;
mod result;
mod timeline;
mod transcript;

pub(super) const OFF: Style = Style { enabled: false, terminal: false };
pub(super) const ON: Style = Style { enabled: true, terminal: true };

pub(super) fn w(id: &str, st: WorkerState) -> WorkerInfo {
    let mut x = WorkerInfo::new(id, "codex", "gpt-x", "write the parser tests", st);
    x.elapsed_ms = 14_000;
    x.tokens = 1200;
    x.cost_usd = Some(0.04);
    x
}
