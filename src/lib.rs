//! A Jev-gated LLM router for Claude models.
//!
//! Two problems a naive router hits, both solved here:
//! 1. **Lost memory on switch** — the model has no memory; the *transcript* does.
//!    [`transcript::Transcript`] is one canonical history that is sent in full to
//!    whichever model a turn routes to, so continuity survives every switch.
//! 2. **Guessed thresholds** — the route-to-cheap confidence cutoff is not guessed;
//!    the `tune` binary sweeps it over a labeled set and reports the savings/miss
//!    curve so you read the number off your own data.

pub use jev_shared as shared;
pub use jev_catalog as catalog;
pub use jev_approval as approval;
pub use jev_changes as changes;
pub use jev_judgment as judgment;
pub use jev_backend as backend;
pub use jev_routing as routing;
pub use jev_providers as providers;
pub use jev_session as session;
pub use jev_orchestration as orchestration;
pub use jev_doctor as doctor;
pub use jev_chat as chat;
pub use jev_turn as turn;
pub use jev_gateway as gateway;
pub use jev_tui as tui;
pub use jev_calibration as calibration;
