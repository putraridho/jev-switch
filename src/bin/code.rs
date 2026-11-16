//! Code mode: a routed coding agent. Each turn runs one provider's coding CLI
//! (Claude Code, Codex, Gemini, DeepSeek through Claude Code, Grok) in your project
//! with tools on. Jev says what kind of task the turn is; the router picks the best
//! model among the providers you can really use, catches it up on what the others
//! did, and fails over to the next one on a limit or error. All of that lives in
//! `jev_router::turn`; this file only reads lines and prints.
//!
//!   TYPESAFE_API_KEY=... cargo run --bin code [project-dir]

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use jev_router::backend::{Event, Usage};
use jev_router::commands::{self, Command};
use jev_router::providers::Runtime;
use jev_router::router::{Router, CODE_STRONG_THRESHOLD};
use jev_router::turn::{Engine, TurnStatus, Ui};
use jev_router::typesafe::TypeSafe;

const HELP: &str = "jev-code: a coding agent that routes each turn across the AI coding tools you have

USAGE:
  jev-code [project-dir]     start in project-dir (default: current directory)

IN SESSION:
  /model auto                let the router choose (default)
  /model <provider>          pin a provider: claude, codex, gemini, grok, deepseek
  /model <provider>:<model>  pin one model, e.g. codex:gpt-6.1-sol
  /model sonnet|opus|haiku   shorthand for the Claude model of that name
  /model                     show the current pin
  /provider auto|off         routing across providers on (default) or off (Claude only)
  /providers                 what was detected, what is cooling down, what was refused
  /providers refresh         probe again (after logging in or adding a key)
  /new                       start a fresh session on every provider
  /quit                      exit (or Ctrl-D)

A provider is used only if it is installed and logged in (or has its key), is not
cooling down after a usage limit, and its billing is allowed. When a turn hits a limit
or fails, the next best model gets the same prompt; the failed attempt's output is
never shown as an answer. Security work never goes to DeepSeek or Grok.

ENVIRONMENT:
  Jev key                    TYPESAFE_API_KEY or JEV_API_KEY, else ~/.jev-router.env
  JEV_PROVIDERS              off starts with routing off (Claude only); default on
  JEV_ALLOW_CREDITS          1 allows models billed as extra credits (e.g. Claude Fable); default off
  JEV_CHAINS                 JSON file replacing the model order per task type
  DEEPSEEK_API_KEY           enables DeepSeek (billed per token; used through the claude CLI)
  JEV_TIMEOUT_MS             longest wait for Jev before routing without it (default 3000)
  CODE_IDLE_TIMEOUT_SECS     stop a turn when the CLI prints nothing for this long (default 900)
  JEV_CODE_LOG               turn log file (default <state dir>/code-turns.jsonl)
  CODE_PERMISSION_MODE       Claude Code permission mode (default acceptEdits)
  CODE_ALLOWED_TOOLS         extra tools Claude Code may use, e.g. \"Bash(cargo test) Bash(git status)\"
  CLAUDE_BIN, CODEX_BIN, GROK_BIN, CODEX_SANDBOX   binary paths and the Codex sandbox
  CODE_STRONG_THRESHOLD      with routing off: Jev reasoning confidence for Opus (default 0.85)
  DOWNGRADE_AFTER            with routing off: easier turns before stepping back to Sonnet (default 2)

Edits are applied without asking, so use it inside a git repo.";

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Prints a turn as it streams: tool calls and notes on stderr, assistant text on stdout.
struct Console;

impl Ui for Console {
    fn route(&mut self, line: &str) {
        eprintln!("  [route] {line}");
    }

    fn event(&mut self, e: &Event) {
        match e {
            Event::Tool(summary) => eprintln!("  \u{23fa} {summary}"),
            // Permission hints start with \u{2717}; they are diagnostics, so they stay on stderr.
            Event::Text(t) if t.starts_with('\u{2717}') => eprintln!("  {t}"),
            Event::Text(t) => println!("{t}\n"),
            _ => {}
        }
    }

    fn note(&mut self, line: &str) {
        for l in line.lines() {
            eprintln!("  {l}");
        }
    }
}

fn report_usage(u: &Usage) {
    eprintln!(
        "  [usage] context {}K \u{b7} turn total: in {}K (cached {}K), out {}\n",
        u.context_tokens.unwrap_or(0) / 1000,
        u.input_tokens / 1000,
        u.cached_input_tokens / 1000,
        u.output_tokens
    );
}

fn indented(text: &str) -> String {
    text.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
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
    // The strong threshold only matters with routing off (the Claude-only tier rule).
    let router = Router::new(TypeSafe::from_env()?, 1.0)
        .with_strong_threshold(env_f64("CODE_STRONG_THRESHOLD", CODE_STRONG_THRESHOLD));
    eprintln!("detecting providers...");
    let runtime = Runtime::start().await;
    let mut engine = Engine::new(runtime, router, dir)
        .with_enabled(commands::routing_enabled(std::env::var("JEV_PROVIDERS").ok().as_deref()))
        .with_downgrade_after(env_f64("DOWNGRADE_AFTER", 2.0) as u32);

    println!("jev-code \u{2014} {}", engine.dir().display());
    println!("{}", indented(&engine.summary_text()));
    println!(
        "  routing: {} \u{b7} /model auto|<provider>[:<model>] \u{b7} /provider auto|off \u{b7} /providers [refresh] \u{b7} /new \u{b7} /quit\n",
        if engine.enabled() { "on" } else { "off (Claude only)" }
    );

    let stdin = io::stdin();
    let mut ui = Console;
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        match commands::parse(&line) {
            Command::Empty => {}
            Command::Quit => break,
            Command::New => {
                engine.new_session();
                println!("  new session\n");
            }
            Command::ShowModel => println!("  model: {}\n", engine.pin()),
            Command::SetPin(pin) => {
                println!("  model: {pin}\n");
                engine.set_pin(pin);
            }
            Command::SetRouting(on) => {
                engine.set_enabled(on);
                println!("  routing: {}\n", if on { "on" } else { "off (Claude only)" });
            }
            Command::Providers { refresh } => {
                if refresh {
                    engine.refresh_providers().await;
                }
                println!("{}\n", indented(&engine.providers_text()));
            }
            Command::Invalid(message) => println!("  {message}\n"),
            Command::Prompt(prompt) => {
                let report = engine.run(&prompt, &mut ui).await;
                match (report.status, &report.usage) {
                    (TurnStatus::Answered, Some(usage)) => report_usage(usage),
                    // Errors and "nothing usable" were already printed as notes; the session stays alive.
                    _ => println!(),
                }
            }
        }
    }
    Ok(())
}
