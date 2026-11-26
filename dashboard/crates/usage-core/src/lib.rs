#![forbid(unsafe_code)]
//! Flashboard's domain (`docs/dashboard/DESIGN.md` section 4): pure data, parsers and policy.
//!
//! No IO, no clock, no runtime and no engine dependency. Types marked "IPC" derive
//! `serde::Serialize` (and `ts_rs::TS` under the `bindings` feature) and may cross to the
//! web UI; types without that mark cannot, by construction.

pub mod alerts;
pub mod assess;
pub mod history;
pub mod ipc;
pub mod label;
pub mod meter;
pub mod parse;
pub mod port;
pub mod provider;
pub mod quantity;
pub mod secret;
pub mod settings;
pub mod summary;
pub mod text;
pub mod time;
pub mod usage;
pub mod view;
