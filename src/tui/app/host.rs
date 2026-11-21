//! Programs on this machine that session commands hand work to: a shell for `!command`,
//! `$EDITOR` for `/edit` and `/memory`, and the clipboard for `/copy`. The choices (which
//! program, how output is cut, how a note for the model reads) are pure and tested; the
//! process calls around them are thin.

pub mod clipboard;
pub mod editor;
pub mod shell;
