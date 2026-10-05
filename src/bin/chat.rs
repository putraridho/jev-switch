//! Interactive router REPL. Each line: Jev routes it, the chosen Claude model
//! answers with the FULL transcript (so memory survives switches), and the turn
//! is appended. Set CHEAP_THRESHOLD to the value the `tune` sweep suggests.
//!
//!   TYPESAFE_API_KEY=... cargo run --bin chat   (Claude runs via Claude Code)

use std::io::{self, Write};

use anyhow::Result;
use jev_router::anthropic::{Anthropic, MODEL_CHEAP, MODEL_MID, MODEL_STRONG};
use jev_router::router::Router;
use jev_router::transcript::Transcript;
use jev_router::typesafe::TypeSafe;

const SYSTEM: &str =
    "You are a concise engineering assistant. Keep continuity across the conversation.";

#[tokio::main]
async fn main() -> Result<()> {
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
        "jev-router — cheap={} mid={} strong={}; cheap≥{} strong≥{}. Ctrl-D to quit.\n",
        MODEL_CHEAP, MODEL_MID, MODEL_STRONG, threshold, strong
    );
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break; // EOF
        }
        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }

        let decision = router.route_turn(prompt, &tx.recent_context(6), prev_model).await?;
        prev_model = Some(decision.model);
        eprintln!(
            "  [route] {} ({}, conf {:.2}) — {}",
            decision.model, decision.kind, decision.confidence, decision.reason
        );

        tx.push_user(prompt);
        // The full transcript goes to whichever model was chosen → memory intact.
        let (reply, usage) = claude
            .complete(decision.model, Some(&tx.system), &tx.messages, 1024)
            .await?;
        tx.push_assistant(&reply);

        println!("{}\n", reply.trim());
        eprintln!(
            "  [usage] in {} out {} cached {}\n",
            usage.input_tokens, usage.output_tokens, usage.cache_read_input_tokens
        );
    }
    Ok(())
}
