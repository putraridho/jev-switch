//! One-shot headless turn through any backend, for evals and scripts.
//!
//!   echo "prompt" | ask <provider> <model> <dir>
//!
//! Prints exactly one JSON line (see `turnlog::ask_json`) and exits 0 even when
//! the model failed (the JSON says so); a non-zero exit means a usage error.
//! `ASK_IDLE_SECS` (default 600) cuts off a CLI that goes quiet. Pass a snapshot
//! directory as `<dir>`: the turn is meant to be read-only.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;

use jev_router::backend::{self, ProviderId, TurnRequest};
use jev_router::{providers, turnlog};

const DEFAULT_IDLE_SECS: u64 = 600;

fn idle_from_env() -> Duration {
    let secs = std::env::var("ASK_IDLE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_IDLE_SECS);
    Duration::from_secs(secs)
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("ask: {msg}\nusage: echo PROMPT | ask <provider> <model> <dir>");
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [provider, model, dir] = args.as_slice() else { return usage_error("expected 3 arguments") };
    let id: ProviderId = match provider.parse() {
        Ok(id) => id,
        Err(e) => return usage_error(&e),
    };
    if !Path::new(dir).is_dir() {
        return usage_error(&format!("`{dir}` is not a directory"));
    }
    let mut prompt = String::new();
    if tokio::io::stdin().read_to_string(&mut prompt).await.is_err() || prompt.trim().is_empty() {
        return usage_error("the prompt on stdin is empty or not UTF-8");
    }
    let req = TurnRequest { dir: dir.into(), model: model.clone(), prompt, session_id: None, mode: Default::default() };
    let started = Instant::now();
    let result = backend::run_turn(&*providers::backend_for(id), &req, idle_from_env(), &mut |_| {}).await;
    let wall_ms = started.elapsed().as_millis() as u64;
    println!("{}", turnlog::ask_json(id.as_str(), model, &result, wall_ms));
    ExitCode::SUCCESS
}
