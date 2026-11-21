//! Tests of the Gemini backend, by subject; the fixtures and fakes are shared here.

use serde_json::Value;
use tokio::process::Command;

use super::*;
use jev_approval::domain::mode::Mode;
use crate::{Event, TurnRequest, Usage};

mod agy;
mod auth;
mod command;
mod errors;
mod stream_events;

    const GEMINI_TURN: &str = fixture!("gemini/turn_tools.jsonl");
    const GEMINI_QUOTA: &str = fixture!("gemini/err_quota.jsonl");
    const GEMINI_EMPTY_ERR: &str = fixture!("gemini/err_empty.jsonl");
    const GEMINI_EMPTY_OK: &str = fixture!("gemini/empty_success.jsonl");
    const GEMINI_AUTH_STDERR: &str = fixture!("gemini/err_auth_stderr.txt");
    const AGY_TURN: &str = fixture!("agy/turn_tools.jsonl");
    const AGY_AUTH: &str = fixture!("agy/err_auth.jsonl");
    const AGY_QUOTA: &str = fixture!("agy/err_quota.jsonl");
    const AGY_EMPTY: &str = fixture!("agy/empty_success.jsonl");
    const AGY_EMPTY_DENIED: &str = fixture!("agy/empty_success_denied.jsonl");

    fn gemini() -> GeminiBackend {
        GeminiBackend::new(Flavor::GeminiCli, "gemini")
    }

    fn agy() -> GeminiBackend {
        GeminiBackend::new(Flavor::Antigravity, "agy")
    }

    fn parse_all(backend: &GeminiBackend, fixture: &str) -> Vec<Event> {
        fixture.lines().flat_map(|l| {
            if backend.flavor == Flavor::Antigravity {
                serde_json::from_str::<Value>(l).map(|ev| agy::agy_events(&ev)).unwrap_or_default()
            } else { backend.parse_line(l) }
        }).collect()
    }

    fn failed_message(events: &[Event]) -> String {
        match events.last() {
            Some(Event::Failed { message }) => message.clone(),
            other => panic!("expected a Failed last, got {other:?}"),
        }
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest {
            dir: "/tmp".into(),
            model: "m-1".into(),
            prompt: "do the thing".into(),
            images: vec![],
            session_id: session.map(String::from),
            mode: Default::default(),
            trust_project_settings: false,
        }
    }
