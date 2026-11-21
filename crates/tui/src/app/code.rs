//! The logic of the `jev-code` binary, so `src/bin/code.rs` is only a root. `entry::run` reads the
//! arguments and picks a mode; `oneshot` (`doctor`, `-p`), `session` and `loops` run it; `setup`
//! builds the engine; `term` knows the terminals; `shell_commands` are `update` and `ext`.

pub mod entry;
mod loops;
mod oneshot;
mod session;
mod setup;
mod shell_commands;
mod term;
