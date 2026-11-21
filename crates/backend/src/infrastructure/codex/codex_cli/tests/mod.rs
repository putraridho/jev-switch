//! Tests of the Codex backend, by subject; the fixtures and fakes are shared here.

use serde_json::json;
use tokio::process::Command;

use super::*;
use crate::Usage;

mod command;
mod errors;
mod models;
mod stream_events;

    const TURN1: &str = fixture!("codex/turn1.jsonl");
    const TURN2: &str = fixture!("codex/turn2_resume.jsonl");
    const ERR_PLAN: &str = fixture!("codex/err_model_not_on_plan.jsonl");

    fn backend() -> CodexBackend {
        CodexBackend::new("codex", "workspace-write")
    }

    fn parse_all(backend: &CodexBackend, fixture: &str) -> Vec<Event> {
        fixture.lines().flat_map(|l| backend.parse_line(l)).collect()
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn req(session: Option<&str>) -> TurnRequest {
        TurnRequest { dir: "/tmp/proj".into(), model: "gpt-x".into(), prompt: "hi".into(), images: vec![], session_id: session.map(String::from), mode: Default::default(), trust_project_settings: false }
    }
