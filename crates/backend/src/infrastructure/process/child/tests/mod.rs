use super::*;
use crate::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod approvals;
mod approvals_pending;
mod elicitation;
mod process_group;
mod prompt_files;
mod run_turn;
mod steering;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("jev-prompt-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
/// A backend whose "CLI" is a shell script printing canned lines, so the runner
/// is tested without any real CLI. Line grammar: `FIN:text`, `FAIL:message`,
/// `TOOL:summary`, `TEXT:text`.
fn fake_cli(script: &'static str) -> crate::testing::ScriptedCli {
    crate::testing::ScriptedCli::new(ProviderId::Claude, script)
}

fn req() -> TurnRequest {
    TurnRequest { dir: std::env::temp_dir(), model: "m".into(), prompt: "hello".into(), images: vec![], session_id: None, mode: Default::default(), trust_project_settings: false }
}

async fn run(script: &'static str, idle_ms: u64) -> (Result<TurnOutcome, TurnError>, Vec<Event>) {
    let mut seen = Vec::new();
    let mut on_event = |e: &Event| seen.push(e.clone());
    let r = run_turn(&fake_cli(script), &req(), Duration::from_millis(idle_ms), &mut on_event).await;
    (r, seen)
}
