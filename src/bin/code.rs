//! Code mode: a routed coding agent. Each turn runs Claude Code (`claude -p`) in
//! your project with tools on, resuming ONE Claude Code session so files read,
//! edits, and command output persist across turns and across model switches.
//! Jev picks the tier per turn; code mode uses two tiers:
//!
//! - Security-sensitive or confidently-reasoning turns, and edits to auth code → Opus.
//! - Everything else → Sonnet. (No Haiku: across five repos its explanations held
//!   up ~85% at every confidence, and its cache-cold turns save little here.)
//! - Upgrades happen immediately; downgrades only after DOWNGRADE_AFTER confident turns
//!   in a row want a lower tier (one tier at a time). Prompt caches are per model and a coding context is
//!   ~100K tokens, so a switch rebuilds the whole cache — one cheap turn on a
//!   cheaper model usually costs more than staying put.
//!
//!   TYPESAFE_API_KEY=... cargo run --bin code [project-dir]
//!
//! Commands: /model sonnet|opus|auto · /new (fresh session) · /quit
//! Env: CODE_STRONG_THRESHOLD (0.85) DOWNGRADE_AFTER (2)
//!      CODE_PERMISSION_MODE (acceptEdits) CODE_ALLOWED_TOOLS (e.g. "Bash(cargo *) Bash(git status)")
//!      CODE_IDLE_TIMEOUT_SECS (900) JEV_CODE_LOG (turn log, default <state dir>/code-turns.jsonl)

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::backend::claude::ClaudeBackend;
use jev_router::backend::{self, Backend, Event, TurnError, TurnOutcome, TurnRequest, Usage};
use jev_router::log;
use jev_router::router::{CodeWant, Router, Sticky, CODE_STRONG_THRESHOLD};
use jev_router::turnlog::turn_row;
use jev_router::typesafe::TypeSafe;

const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];

const HELP: &str = "jev-code: Claude Code with per-turn model routing (Sonnet, or Opus for hard and security work)

USAGE:
  jev-code [project-dir]     start in project-dir (default: current directory)

IN SESSION:
  /model sonnet|opus|auto    pin a model, or hand control back to the router
  /new                       start a fresh Claude Code session
  /quit                      exit (or Ctrl-D)

ENVIRONMENT:
  Jev key                    TYPESAFE_API_KEY or JEV_API_KEY, else ~/.jev-router.env
  CODE_STRONG_THRESHOLD      Jev reasoning confidence for Opus (default 0.85)
  JEV_TIMEOUT_MS             longest wait for Jev before keeping the current model (default 3000)
  DOWNGRADE_AFTER            easier turns before stepping back to Sonnet (default 2)
  CODE_PERMISSION_MODE       Claude Code permission mode (default acceptEdits)
  CODE_ALLOWED_TOOLS         extra allowed tools, e.g. \"Bash(cargo test) Bash(git status)\"
  CLAUDE_BIN                 path to the claude binary (default: claude on PATH)
  CODE_IDLE_TIMEOUT_SECS     stop a turn when the CLI prints nothing for this long (default 900)
  JEV_CODE_LOG               turn log file (default <state dir>/code-turns.jsonl)

Edits are applied without asking, so use it inside a git repo.";

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn short(name: &str) -> &str {
    match name {
        n if n == MODEL_CHEAP => "haiku",
        n if n == MODEL_MID => "sonnet",
        _ => "opus",
    }
}

struct Session {
    dir: PathBuf,
    id: Option<String>,
    sticky: Sticky,
    pinned: Option<usize>,
    context_tokens: u64,
    recent: Vec<String>,
}

impl Session {
    fn new(dir: PathBuf) -> Self {
        Self { dir, id: None, sticky: Sticky::default(), pinned: None, context_tokens: 0, recent: Vec::new() }
    }

    fn recent_context(&self) -> String {
        self.recent.join("\n")
    }

    fn remember(&mut self, user: &str, reply: &str) {
        let reply: String = reply.chars().take(300).collect();
        self.recent.push(format!("user: {user}\nassistant: {reply}"));
        if self.recent.len() > 3 {
            self.recent.remove(0);
        }
    }
}

async fn want(router: &Router, s: &Session, prompt: &str) -> CodeWant {
    let w = router.code_want(prompt, &s.recent_context(), s.sticky.tier.unwrap_or(1)).await;
    if w.reason.starts_with("jev unavailable") {
        // Never crash the session; stay put (or start on Sonnet).
        eprintln!("  [jev] {}; keeping current model", w.reason);
    }
    w
}

/// Apply a /model pin, else the shared upgrade-now, downgrade-slowly rule.
fn decide(s: &mut Session, w: &CodeWant, downgrade_after: u32) -> (usize, String) {
    match s.pinned {
        Some(p) => (p, "pinned with /model".into()),
        None => s.sticky.decide(w, downgrade_after),
    }
}

/// Print progress as the turn streams: tool calls on stderr, assistant text on stdout.
fn show(ev: &Event) {
    match ev {
        Event::Tool(summary) => eprintln!("  \u{23fa} {summary}"),
        // Permission hints start with \u{2717}; they are diagnostics, so they stay on stderr as before.
        Event::Text(t) if t.starts_with('\u{2717}') => eprintln!("  {t}"),
        Event::Text(t) => println!("{t}\n"),
        _ => {}
    }
}

fn report_usage(u: &Usage) {
    eprintln!(
        "  [usage] context {}K \u{b7} turn total: in {}K (cached {}K), out {}",
        u.context_tokens.unwrap_or(0) / 1000,
        u.input_tokens / 1000,
        u.cached_input_tokens / 1000,
        u.output_tokens
    );
}

/// Longest silence from the CLI before the turn is cut off, so a hung CLI cannot
/// freeze the session.
fn idle_timeout() -> Duration {
    Duration::from_secs(env_f64("CODE_IDLE_TIMEOUT_SECS", 900.0) as u64)
}

/// Run one turn through Claude Code, streaming progress.
async fn run_turn(
    backend: &ClaudeBackend,
    s: &Session,
    model: &str,
    prompt: &str,
) -> Result<TurnOutcome, TurnError> {
    let req = TurnRequest {
        dir: s.dir.clone(),
        model: model.to_string(),
        prompt: prompt.to_string(),
        session_id: s.id.clone(),
    };
    backend::run_turn(backend, &req, idle_timeout(), &mut show).await
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(flag) = std::env::args().nth(1).filter(|a| a.starts_with('-')) {
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("jev-code {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => bail!("unknown option {other} (try --help)"),
        }
    }
    let dir = match std::env::args().nth(1) {
        Some(d) => PathBuf::from(d),
        None => std::env::current_dir()?,
    }
    .canonicalize()
    .context("project dir")?;
    // Stricter than chat: Sonnet's hard-debugging misses sat at reasoning 0.78-0.83.
    // The cheap cutoff is unused here (code mode has no Haiku tier).
    let router = Router::new(TypeSafe::from_env()?, 1.0)
        .with_strong_threshold(env_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD));
    let downgrade_after = env_f64("DOWNGRADE_AFTER", 2.0) as u32;
    let mut s = Session::new(dir);
    let backend = ClaudeBackend::claude();
    let turn_log = log::path_from_env("JEV_CODE_LOG", "code-turns.jsonl");

    println!(
        "jev-code — {}\n  sonnet / opus · /model sonnet|opus|auto · /new · /quit\n",
        s.dir.display()
    );
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let prompt = line.trim();
        match prompt {
            "" => continue,
            "/quit" | "/exit" => break,
            "/new" => {
                s = Session::new(s.dir.clone());
                println!("  new session\n");
                continue;
            }
            _ if prompt.starts_with("/model") => {
                s.pinned = match prompt.split_whitespace().nth(1) {
                    Some("sonnet") => Some(1),
                    Some("opus") => Some(2),
                    _ => None,
                };
                println!("  model: {}\n", s.pinned.map(|t| short(TIERS[t])).unwrap_or("auto"));
                continue;
            }
            _ => {}
        }

        let w = want(&router, &s, prompt).await;
        let (tier, reason) = decide(&mut s, &w, downgrade_after);
        eprintln!("  [route] {} — {}", short(TIERS[tier]), reason);
        let started = Instant::now();
        let result = run_turn(&backend, &s, TIERS[tier], prompt).await;
        let wall_ms = started.elapsed().as_millis() as u64;
        if let Ok(out) = &result {
            s.id = out.session_id.clone().or(s.id.take());
        }
        if let Some(path) = &turn_log {
            let row = turn_row(backend.provider(), TIERS[tier], s.id.as_deref(), &reason, &w.signals, &result, wall_ms);
            log::append(path, &row);
        }
        match result {
            Ok(out) => {
                s.context_tokens = out.usage.context_tokens.unwrap_or(0);
                report_usage(&out.usage);
                s.sticky.tier = Some(tier);
                s.remember(prompt, &out.text);
            }
            // Keep the session alive; the user can retry or switch with /model.
            Err(e) => eprintln!("  [error] {e}\n"),
        }
    }
    Ok(())
}
