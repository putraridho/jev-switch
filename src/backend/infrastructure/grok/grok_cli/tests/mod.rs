//! Tests of the Grok backend, by subject; the fixtures and helpers are shared here.

use tokio::process::Command;

use super::*;
use crate::approval::domain::mode::Mode;
use crate::backend::{Event, Usage};

mod auth;
mod command;
mod errors;
mod stream_events;

pub(super) const TURN1: &str = fixture!("grok/turn1.jsonl");
pub(super) const ERR_AUTH: &str = fixture!("grok/err_auth.jsonl");
pub(super) const ERR_PLAN: &str = fixture!("grok/err_not_entitled.jsonl");
pub(super) const EMPTY: &str = fixture!("grok/empty.jsonl");

pub(super) fn backend() -> GrokBackend {
    GrokBackend::new("grok")
}

pub(super) fn parse_all(fixture: &str) -> Vec<Event> {
    fixture.lines().flat_map(|l| backend().parse_line(l)).collect()
}

pub(super) fn args(cmd: &Command) -> Vec<String> {
    cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

pub(super) fn req(session: Option<&str>) -> TurnRequest {
    TurnRequest {
        dir: "/tmp".into(),
        model: "grok-4.6".into(),
        prompt: "hi".into(),
        images: vec![],
        session_id: session.map(String::from),
        mode: Default::default(),
        trust_project_settings: false,
    }
}
