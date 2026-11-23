//! Owns `jev-code doctor` and `jev-code -p`. Must not run a session.

use std::io::Read;
use std::path::{Path, PathBuf};
use crate::cli_args as cli;
use crate::cli_args::{Args, PromptSource};
use crate::console::terminal_ui::Console;
use jev_shared::infrastructure::jsonl_log as log;
use crate::repl::session::turn_race::SigInt;
use crate::repl::session::Repl;
use crate::views::status_facts as views;
use super::setup::{engine_for, jev_client, router_for, start_runtime, ProbeMode};
use super::term::Term;

// ----------------------------------------------------------------------- doctor

/// `jev-code doctor`: the report on stdout; exit 1 when something is a Problem.
pub(super) async fn doctor(dir: &Path, console: &Console) -> i32 {
    let (mut runtime, _) = start_runtime(console, ProbeMode::Live).await;
    runtime.attach_project(dir);
    // The capability handshake sends no prompt and spends no quota; a recent answer is reused.
    let handshake = runtime.handshake(false).await;
    let mut inputs = jev_doctor::infrastructure::gather_inputs::gather(&runtime, dir).await;
    inputs.handshake = handshake;
    let report = jev_doctor::domain::diagnose::diagnose(&inputs);
    console.out(&views::doctor::doctor_text(&report, console.out_style(), console.width()));
    i32::from(views::doctor::has_problem(&report))
}

// --------------------------------------------------------------------- one-shot

/// `jev-code -p`: one turn. The answer alone on stdout; exit 0 answered, 1 failed, 130 stopped.
pub(super) async fn print_mode(args: &Args, source: &PromptSource, dir: &Path, term: &Term, console: Console, state_dir: Option<PathBuf>) -> i32 {
    let prompt = match source {
        PromptSource::Text(t) => t.clone(),
        PromptSource::Stdin => {
            let mut text = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut text) {
                eprintln!("jev-code: cannot read the prompt from stdin: {e}");
                return 2;
            }
            text
        }
    };
    if prompt.trim().is_empty() {
        eprintln!("jev-code: no prompt given (nothing on stdin)\n{}", cli::USAGE);
        return 2;
    }
    let router = match jev_client(term.err_style) {
        Ok(ts) => router_for(ts),
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let (runtime, _) = start_runtime(&console, ProbeMode::Cached).await;
    // A one-shot starts a fresh conversation and leaves the saved session alone, unless -c asks to continue it.
    let persist = if args.resume { state_dir.clone() } else { None };
    // The privacy rules, budget and failover settings apply to a one-shot too: they are read from
    // the state folder even when the session itself is not kept.
    let engine = engine_for(args, runtime, router, dir.to_path_buf(), console.can_ask(), None).with_persistence_and_settings(persist, state_dir.as_deref());
    let mut repl = Repl::new(engine, console.clone(), std::env::var("HOME").ok(), false);
    repl.enforce_team_limit();
    if args.resume {
        resume_quietly(&mut repl, &console);
    }
    let mut sigint = match SigInt::install() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("jev-code: cannot listen for Ctrl-C: {e}");
            return 1;
        }
    };
    match &args.repeat {
        Some(repeat) => repl.repeat_shot(&prompt, args.format, repeat, &mut sigint, &mut crate::repl::headless::sleeper()).await,
        None => repl.one_shot_as(&prompt, args.format, &mut sigint).await,
    }
}

/// `-c`: load the saved session and say so on stderr; nothing to continue is not an error.
pub(super) fn resume_quietly(repl: &mut Repl, console: &Console) {
    let style = console.err_style();
    match repl.engine.resume() {
        Ok(info) => console.err(&format!("  {}", views::startup::resumed_line(&info, log::now_ms(), style))),
        Err(e) => console.err(&format!("  {}", style.dim(&format!("starting fresh: {e}")))),
    }
}
