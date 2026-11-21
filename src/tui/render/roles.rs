//! How each kind of thing in the interactive conversation looks, so a glance tells them
//! apart:
//!
//! - what **you** sent: a shaded band with `❯` (a rule and `❯` without colour);
//! - **tool calls**: dim, under a `⏺` marker, with what came back on a `⎿` row under it and
//!   an edit's lines as a small diff;
//! - the model's **narration** between tool calls: plain bright text, it is a message to
//!   you, set apart by a blank line and indented with the tool rows;
//! - the **answer**: plain bright text, set apart by a blank line.
//!
//! Colour is never the only cue: each role also has its own marker, indent or weight.
pub mod edit;
pub mod group;
pub mod prose;
pub mod result;
pub mod sent;
pub mod tool;

#[cfg(test)]
mod tests;
