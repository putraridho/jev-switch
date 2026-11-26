#![forbid(unsafe_code)]
//! Flashboard's IO shell (`docs/dashboard/DESIGN.md` sections 5 to 8): the adapters behind
//! core's ports, the history store and the one service loop.
//!
//! Three rules hold for the whole crate: only `engine_bridge` names the engine, only `proc`
//! starts a process, and only `http` talks to the network. Errors are closed enums; the one
//! diagnostic channel is `diag::emit`.

pub mod clock;
pub mod diag;
pub mod engine_bridge;
pub mod envfile;
pub mod fsutil;
pub mod http;
pub mod probe;
pub mod proc;
pub mod registry;
pub mod secrets;
pub mod service;
pub mod settings_file;
pub mod sources;
pub mod store;
pub mod tail;
pub mod timer;
pub mod watch;
