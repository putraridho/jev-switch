//! The keys the editor binds: Shift-Tab cycles the mode, Enter after a backslash continues the line.

use rustyline::{Cmd, ConditionalEventHandler, EventContext, Movement, RepeatCount};

use crate::approval::domain::mode::SharedMode;

/// Shift-Tab: the next mode, shown at once.
pub(super) struct CycleMode(pub(super) SharedMode);

impl ConditionalEventHandler for CycleMode {
    fn handle(&self, _: &rustyline::Event, _: RepeatCount, _: bool, _: &EventContext) -> Option<Cmd> {
        self.0.cycle();
        Some(Cmd::Repaint)
    }
}

/// Enter right after a `\\` at the end of the line: the backslash becomes a new line, as
/// in a shell. Anywhere else Enter sends the line.
pub(super) struct BackslashNewline;

impl ConditionalEventHandler for BackslashNewline {
    fn handle(&self, _: &rustyline::Event, _: RepeatCount, _: bool, ctx: &EventContext) -> Option<Cmd> {
        continues_line(ctx.line(), ctx.pos()).then(|| Cmd::Replace(Movement::BackwardChar(1), Some("\n".to_string())))
    }
}

/// Whether Enter at `pos` continues the line: the cursor is at the end, after one `\\`
/// (a doubled one is a literal backslash).
pub fn continues_line(line: &str, pos: usize) -> bool {
    pos == line.len() && line.ends_with('\\') && !line.ends_with("\\\\")
}
