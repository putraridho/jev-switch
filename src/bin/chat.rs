//! Interactive router REPL. Each line: Jev routes it, the chosen Claude model
//! answers with the FULL transcript (so memory survives switches), and the turn
//! is appended. Set CHEAP_THRESHOLD to the value the `tune` sweep suggests.
//!
//!   jev-chat   (installed by scripts/install.sh; Claude runs via Claude Code)

use std::io::{self, Write};

use anyhow::Result;
use jev_router::anthropic::{Anthropic, MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::Router;
use jev_router::transcript::Transcript;
use jev_router::typesafe::TypeSafe;

const HELP: &str = "jev-chat: Q&A chat with per-turn model routing (Haiku for simple, Sonnet by default, Opus for hard or security questions)

USAGE:
  jev-chat

IN SESSION:
  /new          clear the conversation
  /quit         exit (or Ctrl-D)

ENVIRONMENT:
  Jev key            TYPESAFE_API_KEY or JEV_API_KEY, else ~/.jev-router.env
  CHEAP_THRESHOLD    Jev simple confidence for Haiku (default 0.50)
  STRONG_THRESHOLD   Jev reasoning confidence for Opus (default 0.85)
  CLAUDE_BIN         path to the claude binary (default: claude on PATH)

Runs through Claude Code (claude -p) on your Claude subscription. No file or tool
access: for work in a project, use jev-code.";

const SYSTEM: &str =
    "You are a concise engineering assistant. Keep continuity across the conversation.";

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(flag) = std::env::args().nth(1) {
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("jev-chat {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other} (try --help)"),
        }
    }
    let threshold: f64 = std::env::var("CHEAP_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.50); // calibrated: `tune` + 1000-prompt run

    // Sonnet matched Opus on every sampled prompt below this; see README.
    let strong: f64 = std::env::var("STRONG_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.85);

    let claude = Anthropic::from_env()?;
    let router = Router::new(TypeSafe::from_env()?, threshold).with_strong_threshold(strong);
    let mut tx = Transcript::new(SYSTEM);
    let mut prev_model: Option<&'static str> = None;

    println!(
        "jev-chat — haiku / sonnet / opus (cheap≥{} strong≥{}) · /new · /quit\n",
        threshold, strong
    );
    let _ = (MODEL_CHEAP, MODEL_STRONG);
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let prompt = line.trim();
        match prompt {
            "" => continue,
            "/quit" | "/exit" => break,
            "/new" => {
                tx = Transcript::new(SYSTEM);
                prev_model = None;
                println!("  conversation cleared\n");
                continue;
            }
            _ => {}
        }

        // Never end the session on an error: Jev down → Sonnet (the fallback tier).
        let (model, label) = match router.route_turn(prompt, &tx.recent_context(6), prev_model).await {
            Ok(d) => (d.model, format!("{} ({}, conf {:.2}) — {}", d.model, d.kind, d.confidence, d.reason)),
            Err(e) => (MODEL_MID, format!("{MODEL_MID} — jev unavailable ({e:#}), using the fallback")),
        };
        eprintln!("  [route] {label}");

        tx.push_user(prompt);
        // The full transcript goes to whichever model was chosen → memory intact.
        let (reply, usage) = match claude.complete(model, Some(&tx.system), &tx.messages, 1024).await {
            Ok(r) => r,
            Err(e) => {
                // Drop the unanswered message so the transcript stays consistent; retry by resending.
                tx.messages.pop();
                eprintln!("  [error] {e:#}\n  (conversation kept; send the message again to retry)\n");
                continue;
            }
        };
        prev_model = Some(model);
        tx.push_assistant(&reply);

        println!("{}\n", reply.trim());
        eprintln!(
            "  [usage] in {} out {} cached {}\n",
            usage.input_tokens, usage.output_tokens, usage.cache_read_input_tokens
        );
    }
    Ok(())
}
