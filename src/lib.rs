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
pub mod attach;
pub mod availability;
pub mod base64;
pub mod backend;
pub mod budget;
pub mod catalog;
pub mod changes;
pub mod cli;
pub mod commands;
pub mod console;
pub mod cooldown;
pub mod delegate;
pub mod doctor;
pub mod envfile;
pub mod failover;
pub mod front;
pub mod guard;
pub mod guard_cmd;
pub mod handoff;
pub mod host;
pub mod lineedit;
pub mod log;
pub mod mode;
pub mod mcp;
pub mod picker;
pub mod orchestrate;
pub mod prefs;
pub mod permissions;
pub mod prefix;
pub mod privacy;
pub mod providers;
pub mod repl;
pub mod router;
pub mod routing;
pub mod routing_rules;
pub mod scoreboard;
pub mod secrets;
pub mod sessionlog;
pub mod terminal;
pub mod termsetup;
pub mod transcript;
pub mod tui;
pub mod turn;
pub mod turnlog;
pub mod typeahead;
pub mod typesafe;
pub mod uifmt;
pub mod usage;
pub mod views;

