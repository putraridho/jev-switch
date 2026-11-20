//! A Jev-gated LLM router for Claude models.
//!
//! Two problems a naive router hits, both solved here:
//! 1. **Lost memory on switch** — the model has no memory; the *transcript* does.
//!    [`transcript::Transcript`] is one canonical history that is sent in full to
//!    whichever model a turn routes to, so continuity survives every switch.
//! 2. **Guessed thresholds** — the route-to-cheap confidence cutoff is not guessed;
//!    the `tune` binary sweeps it over a labeled set and reports the savings/miss
//!    curve so you read the number off your own data.

#[cfg(test)]
#[macro_use]
pub mod testkit;
pub mod approval;
pub mod backend;
pub mod calibration;
pub mod catalog;
pub mod changes;
pub mod chat;
pub mod doctor;
pub mod gateway;
pub mod judgment;
pub mod orchestration;
pub mod providers;
pub mod routing;
pub mod session;
pub mod shared;
pub mod tui;
pub mod turn;
