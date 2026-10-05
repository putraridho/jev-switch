//! Code mode: a routed coding agent. Each turn runs Claude Code (`claude -p`) in
//! your project with tools on, resuming ONE Claude Code session so files read,
//! edits, and command output persist across turns and across model switches.
//! Jev picks the tier per turn; this policy adapts it for agentic coding:
//!
//! - Security-sensitive or confidently-reasoning turns → Opus.
//! - Confidently-simple, read-only turns while the context is small → Haiku,
//!   restricted to read-only tools (Read/Grep/Glob) so it can't edit.
//! - Everything else, and every turn that edits or runs commands → Sonnet minimum.
//! - Upgrades happen immediately; downgrades only after DOWNGRADE_AFTER confident turns
//!   in a row want a lower tier (one tier at a time). Prompt caches are per model and a coding context is
//!   ~100K tokens, so a switch rebuilds the whole cache — one cheap turn on a
//!   cheaper model usually costs more than staying put.
//!
//!   TYPESAFE_API_KEY=... cargo run --bin code [project-dir]
//!
//! Commands: /model haiku|sonnet|opus|auto · /new (fresh session) · /quit
//! Env: CHEAP_THRESHOLD (0.50) STRONG_THRESHOLD (0.85) DOWNGRADE_AFTER (2)
//!      CODE_PERMISSION_MODE (acceptEdits) CODE_ALLOWED_TOOLS (e.g. "Bash(cargo *) Bash(git status)")

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use jev_router::anthropic::{MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::{is_security_sensitive, Router};
use jev_router::typesafe::TypeSafe;

const TIERS: [&str; 3] = [MODEL_CHEAP, MODEL_MID, MODEL_STRONG];
const READ_ONLY_TOOLS: &str = "Read,Grep,Glob";
/// Above this many context tokens the cheap model is skipped: Haiku's window is
/// 200K, and a big context is exactly where a cache rebuild costs the most.
const CHEAP_MAX_CONTEXT: u64 = 120_000;
/// `needs_changes` probability at or above which a turn counts as an edit turn.
const EDIT_P: f64 = 0.5;

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

/// What Jev's judgment asks for this turn, before stickiness.
struct Want {
    tier: usize,
    confident: bool,
    reason: String,
}

struct Session {
    dir: PathBuf,
    id: Option<String>,
    tier: Option<usize>,
    /// Consecutive turns that wanted a lower tier than the current one.
    lower_streak: u32,
    pinned: Option<usize>,
    context_tokens: u64,
    recent: Vec<String>,
}

impl Session {
    fn new(dir: PathBuf) -> Self {
        Self { dir, id: None, tier: None, lower_streak: 0, pinned: None, context_tokens: 0, recent: Vec::new() }
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

async fn want(router: &Router, s: &Session, prompt: &str) -> Want {
    if is_security_sensitive(prompt) {
        return Want { tier: 2, confident: true, reason: "security-sensitive".into() };
    }
    let ctx = s.recent_context();
    let (ans, edit_p) = match tokio::join!(router.classify(prompt, &ctx), router.needs_changes(prompt, &ctx)) {
        (Ok(a), Ok(p)) => (a, p),
        (Err(e), _) | (_, Err(e)) => {
            // Jev unavailable: never crash the session; stay put (or start on Sonnet).
            eprintln!("  [jev] unavailable ({e:#}); keeping current model");
            return Want { tier: s.tier.unwrap_or(1), confident: false, reason: "jev unavailable".into() };
        }
    };
    let edit = edit_p >= EDIT_P;
    let tag = format!("{} {:.2}, edit p {:.2}", ans.choice, ans.confidence, edit_p);
    if ans.choice == "reasoning" && ans.confidence >= router.strong_threshold {
        return Want { tier: 2, confident: true, reason: format!("confidently reasoning ({tag})") };
    }
    if ans.choice == "simple" && ans.confidence >= router.cheap_threshold {
        if edit {
            return Want { tier: 1, confident: true, reason: format!("simple but edits ({tag})") };
        }
        if s.context_tokens > CHEAP_MAX_CONTEXT {
            return Want { tier: 1, confident: true, reason: format!("simple, context too big for haiku ({tag})") };
        }
        return Want { tier: 0, confident: true, reason: format!("simple, read-only ({tag})") };
    }
    Want { tier: 1, confident: ans.choice == "standard" && ans.confidence >= 0.5, reason: format!("everyday ({tag})") }
}

/// Apply stickiness: upgrade now, downgrade only after a streak.
fn decide(s: &mut Session, w: &Want, downgrade_after: u32) -> (usize, String) {
    if let Some(p) = s.pinned {
        return (p, "pinned with /model".into());
    }
    let Some(cur) = s.tier else {
        return (w.tier, w.reason.clone());
    };
    if w.tier > cur {
        s.lower_streak = 0;
        return (w.tier, format!("upgrade: {}", w.reason));
    }
    if w.tier == cur || !w.confident {
        s.lower_streak = 0;
        return (cur, format!("stay (warm cache): {}", w.reason));
    }
    s.lower_streak += 1;
    if s.lower_streak >= downgrade_after {
        s.lower_streak = 0;
        // Step down one tier at a time.
        return (cur - 1, format!("downgrade after {downgrade_after} easier turns: {}", w.reason));
    }
    (cur, format!("stay ({}/{} toward downgrade): {}", s.lower_streak, downgrade_after, w.reason))
}

fn tool_summary(name: &str, input: &Value) -> String {
    let field = ["file_path", "command", "pattern", "path", "url", "description"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    match field {
        Some(f) => format!("{name} {}", f.chars().take(100).collect::<String>()),
        None => name.to_string(),
    }
}

/// Run one turn through Claude Code, streaming progress. Returns the final text.
async fn run_turn(s: &mut Session, tier: usize, prompt: &str) -> Result<String> {
    let model = TIERS[tier];
    let mode = std::env::var("CODE_PERMISSION_MODE").unwrap_or_else(|_| "acceptEdits".into());
    let mut cmd = Command::new(std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".into()));
    cmd.args(["-p", "--model", model, "--output-format", "stream-json", "--verbose"])
        .current_dir(&s.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(id) = &s.id {
        cmd.args(["--resume", id]);
    }
    if tier == 0 {
        cmd.args(["--tools", READ_ONLY_TOOLS]);
    } else {
        cmd.args(["--permission-mode", &mode]);
        if let Ok(allowed) = std::env::var("CODE_ALLOWED_TOOLS") {
            cmd.args(["--allowedTools", &allowed]);
        }
    }

    let mut child = cmd.spawn().context("failed to run `claude` (is Claude Code installed?)")?;
    let mut stdin = child.stdin.take().context("claude stdin")?;
    stdin.write_all(prompt.as_bytes()).await?;
    drop(stdin);

    let mut lines = BufReader::new(child.stdout.take().context("claude stdout")?).lines();
    let mut final_text = String::new();
    let mut got_result = false;
    while let Some(line) = lines.next_line().await? {
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        match ev["type"].as_str() {
            Some("assistant") => {
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    match b["type"].as_str() {
                        Some("tool_use") => eprintln!(
                            "  \u{23fa} {}",
                            tool_summary(b["name"].as_str().unwrap_or("?"), &b["input"])
                        ),
                        Some("text") => {
                            let t = b["text"].as_str().unwrap_or("").trim();
                            if !t.is_empty() {
                                println!("{t}\n");
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("system") if ev["subtype"] == "permission_denied" => eprintln!(
                "  \u{2717} denied {} — allow it via CODE_ALLOWED_TOOLS or your Claude Code settings",
                ev["tool_name"].as_str().unwrap_or("tool")
            ),
            Some("result") => {
                got_result = true;
                if let Some(id) = ev["session_id"].as_str() {
                    s.id = Some(id.to_string());
                }
                // `usage` sums every API call in the turn; the context size is the
                // last call's input (fresh + cache read + cache written).
                let u = &ev["usage"];
                let last = u["iterations"].as_array().and_then(|a| a.last()).unwrap_or(u);
                let n = |v: &Value, k: &str| v[k].as_u64().unwrap_or(0);
                let input = |v: &Value| {
                    n(v, "input_tokens") + n(v, "cache_read_input_tokens") + n(v, "cache_creation_input_tokens")
                };
                s.context_tokens = input(last);
                eprintln!(
                    "  [usage] context {}K · turn total: in {}K (cache read {}K, written {}K), out {}",
                    s.context_tokens / 1000,
                    input(u) / 1000,
                    n(u, "cache_read_input_tokens") / 1000,
                    n(u, "cache_creation_input_tokens") / 1000,
                    n(u, "output_tokens")
                );
                final_text = ev["result"].as_str().unwrap_or("").to_string();
                if ev["is_error"].as_bool() == Some(true) {
                    bail!("claude reported an error: {final_text}");
                }
            }
            _ => {}
        }
    }
    let status = child.wait().await?;
    if !got_result {
        bail!("claude exited ({status}) without a result");
    }
    Ok(final_text)
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = match std::env::args().nth(1) {
        Some(d) => PathBuf::from(d),
        None => std::env::current_dir()?,
    }
    .canonicalize()
    .context("project dir")?;
    let router = Router::new(TypeSafe::from_env()?, env_f64("CHEAP_THRESHOLD", 0.50))
        .with_strong_threshold(env_f64("STRONG_THRESHOLD", 0.85));
    let downgrade_after = env_f64("DOWNGRADE_AFTER", 2.0) as u32;
    let mut s = Session::new(dir);

    println!(
        "jev-router code mode — {}\n  haiku (read-only) / sonnet / opus · /model haiku|sonnet|opus|auto · /new · /quit\n",
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
                    Some("haiku") => Some(0),
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
        match run_turn(&mut s, tier, prompt).await {
            Ok(text) => {
                s.tier = Some(tier);
                s.remember(prompt, &text);
            }
            // Keep the session alive; the user can retry or switch with /model.
            Err(e) => eprintln!("  [error] {e:#}\n"),
        }
    }
    Ok(())
}
