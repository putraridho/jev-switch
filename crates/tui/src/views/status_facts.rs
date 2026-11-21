//! The facts behind each screen of `jev-code`, taken from the engine and its runtime and
//! handed to the formatters in [`crate::render`]. Everything here is a pure function of its
//! arguments (the clock and the home directory are passed in), so the binary and the REPL
//! only decide WHEN to print and each mapping is a unit test.
//!
//! One module per family: `providers`, `turns`, `startup`, `prompt`, `doctor`, `pager`.





pub mod doctor;
pub mod pager;
pub mod prompt;
pub mod providers;
pub mod startup;
pub mod turns;
#[cfg(test)]
mod tests;
