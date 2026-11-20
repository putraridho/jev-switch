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

pub mod agentview;
pub mod anthropic;
pub mod attach;
pub mod audit;
pub mod availability;
pub mod background;
pub mod balance;
pub mod base64;
pub mod bgquery;
pub mod backend;
pub mod budget;
pub mod bench;
pub mod bridge;
pub mod catalog;
pub mod changes;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod console;
pub mod cooldown;
pub mod ctxintel;
pub mod delegate;
pub mod workercheck;
pub mod diffcmd;
pub mod diffparse;
pub mod doctor;
pub mod effort;
pub mod failover;
pub mod ext;
pub mod front;
pub mod gimmicks;
pub mod guard;
pub mod guard_cmd;
pub mod export;
pub mod extcmds;
pub mod gitflow;
pub mod handoff;
pub mod headless;
pub mod layers;
pub mod schedule;
pub mod host;
pub mod learn;
pub mod libcmd;
pub mod inflight;
pub mod lineedit;
pub mod shared;
pub use shared::{envfile, log};
pub mod mode;
pub mod mcp;
pub mod picker;
pub mod planstore;
pub mod orchestrate;
pub mod perf;
pub mod probecache;
pub mod prefs;
pub mod permissions;
pub mod prefix;
pub mod privacy;
pub mod projcmd;
pub mod projectkind;
pub mod projmemory;
pub mod providers;
pub mod repomap;
pub mod profile;
pub mod redact;
pub mod repl;
pub mod replay;
pub mod router;
pub mod routing;
pub mod routing_rules;
pub mod scoreboard;
pub mod secrets;
pub mod selfupdate;
pub mod sessionimport;
pub mod sessionlib;
pub mod sessionlog;
pub mod sessionmeta;
pub mod team;
pub mod terminal;
pub mod termenv;
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
pub mod workstation;

