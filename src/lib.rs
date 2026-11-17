//! A Jev-gated LLM router for Claude models.
//!
//! Two problems a naive router hits, both solved here:
//! 1. **Lost memory on switch** — the model has no memory; the *transcript* does.
//!    [`transcript::Transcript`] is one canonical history that is sent in full to
//!    whichever model a turn routes to, so continuity survives every switch.
//! 2. **Guessed thresholds** — the route-to-cheap confidence cutoff is not guessed;
//!    the `tune` binary sweeps it over a labeled set and reports the savings/miss
//!    curve so you read the number off your own data.

pub mod anthropic;
pub mod availability;
pub mod backend;
pub mod catalog;
pub mod changes;
pub mod cli;
pub mod commands;
pub mod console;
pub mod cooldown;
pub mod delegate;
pub mod doctor;
pub mod envfile;
pub mod handoff;
pub mod lineedit;
pub mod log;
pub mod orchestrate;
pub mod prefix;
pub mod providers;
pub mod repl;
pub mod router;
pub mod routing;
pub mod terminal;
pub mod transcript;
pub mod turn;
pub mod turnlog;
pub mod typesafe;
pub mod uifmt;
pub mod usage;
pub mod views;

