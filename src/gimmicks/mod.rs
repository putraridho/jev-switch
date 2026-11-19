//! Small delights that also save time or money. Every module here is pure (plain facts in,
//! a value or a string out), so each is a unit test; the REPL decides where to call them.
//!
//! - [`gauge`]: the quota fuel gauge and the "switch provider" suggestion;
//! - [`savings`]: the savings ticker and the `/wrapped` recap;
//! - [`shortcuts`]: `!` shell passthrough and `#` notes to project memory;
//! - [`ghost`]: inline completion from history and project files;
//! - [`suggest`]: the next-prompt suggestion an answer's closing offer gives;
//! - [`title`]: title-bar status and the bell, so a long turn can be left alone;
//! - [`session_title`]: automatic session names and the "where we left off" line;
//! - [`autopsy`]: `/why-failed`;
//! - [`palette`]: fuzzy search for the Ctrl-K command palette;
//! - [`race`]: `/race` side-by-side rendering and winner choice, `/ask-other` prompts;
//! - [`packs`]: theme packs.

pub mod autopsy;
pub mod gauge;
pub mod ghost;
pub mod packs;
pub mod palette;
pub mod race;
pub mod savings;
pub mod session_title;
pub mod shortcuts;
pub mod suggest;
pub mod title;
