//! `jev-code`: a routed coding agent. Each turn runs one provider's coding CLI (Claude Code,
//! Codex, Gemini, DeepSeek through Claude Code, Grok) in your project with tools on. Jev says
//! what kind of task the turn is; the router picks the best model among the providers you can
//! really use, catches it up on what the others did, and fails over to the next one on a limit
//! or an error.
//!
//! The logic lives in `jev_router::tui::app::code`; this file only starts it. Every exit goes
//! through `run`'s return value, so the terminal guard always runs.
//!
//!   jev-code [dir]            interactive session
//!   jev-code -p "prompt"      one turn, only the answer on stdout
//!   jev-code doctor           check the setup

#[tokio::main]
async fn main() {
    let code = jev_router::tui::app::code::entry::run().await;
    // Every guard is dropped by now. Leave without waiting for a thread blocked on stdin.
    std::process::exit(code);
}
