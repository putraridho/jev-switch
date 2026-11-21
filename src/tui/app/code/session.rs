//! Owns an interactive or piped session: its startup, the banner and the choice of loop.

use std::io::Write;
use std::path::{Path, PathBuf};
use crate::tui::cli_args::Args;
use crate::tui::console::terminal_ui::{Console, CLEAR_SCREEN};
use crate::tui::console::front::FrontMsg;
use crate::session::infrastructure::session_file;
use crate::tui::input::line_editor as lineedit;
use crate::shared::infrastructure::jsonl_log as log;
use crate::tui::app::preferences::Saved;
use crate::tui::app::startup_timeline::{Phase, Timeline};
use crate::tui::repl::session::turn_race::SigInt;
use crate::tui::repl::session::Repl;
use crate::tui::render as uifmt;
use crate::tui::views::status_facts as views;
use super::loops::{edit_loop, pipe_loop};
use super::setup::{engine_for, jev_client, router_for, start_runtime, ProbeMode};
use super::term::Term;

/// The front end's end of the channel from the session.
pub(super) type FrontRx = tokio::sync::mpsc::UnboundedReceiver<FrontMsg>;

pub(super) async fn session(args: &Args, dir: &Path, term: &Term, console: Console, state_dir: Option<PathBuf>, front_rx: Option<FrontRx>) -> i32 {
    // Start on a clean screen: what the shell showed before is not part of this session. Only
    // on a person's terminal and only with colour on, the rule the console's own clear follows;
    // a pipe or `--no-color` must never get escape bytes.
    if term.interactive() && term.out_style.enabled {
        let mut out = std::io::stdout();
        let _ = out.write_all(CLEAR_SCREEN.as_bytes());
        let _ = out.flush();
    }
    let mut timeline = Timeline::start(args.trace);
    let router = match timeline.time(Phase::JevClient, || jev_client(term.err_style)) {
        Ok(ts) => router_for(ts),
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let began = timeline.now();
    let (runtime, probes) = start_runtime(&console, ProbeMode::Cached).await;
    timeline.set_probe_source(probes);
    timeline.record(Phase::ProviderProbe, began);
    let saved = timeline.time(Phase::SavedState, || state_dir.as_deref().map(Saved::load).unwrap_or_default());
    // What the person last chose with `/thinking` beats the environment variable.
    if let Some(on) = saved.thinking {
        console.set_show_thinking(on);
    }
    let home = std::env::var("HOME").ok();
    let mut repl = timeline.time(Phase::Engine, || {
        let engine = engine_for(args, runtime, router, dir.to_path_buf(), console.can_ask(), Some(saved)).with_persistence(state_dir.clone());
        // The interactive screen shows text as it arrives; plain output keeps it until it is final.
        let engine = if front_rx.is_some() { engine.with_streaming() } else { engine };
        Repl::new(engine, console.clone(), home.clone(), term.stdout_tty).with_state_dir(state_dir.clone())
    });
    repl.enforce_team_limit();
    timeline.time(Phase::Banner, || startup(&mut repl, args, term, &console, state_dir.as_deref(), home.as_deref()));
    if args.startup_profile {
        console.out(&timeline.report());
        return 0;
    }
    recheck_slow_providers(&mut repl, &console);

    let mut sigint = match SigInt::install() {
        Ok(s) => s,
        Err(e) => {
            console.err(&format!("jev-code: cannot listen for Ctrl-C: {e}"));
            return 1;
        }
    };
    if let Some(rx) = front_rx {
        // The front end saves the session when it ends.
        let opts = crate::tui::app::frontend_loop::Options { style: term.out_style, history: state_dir.as_deref().map(lineedit::history_path) };
        return crate::tui::app::frontend_loop::run(repl, rx, opts).await;
    }
    let code = if term.interactive() {
        edit_loop(&mut repl, term, &console, state_dir.as_deref(), &mut sigint).await
    } else {
        pipe_loop(&mut repl, &mut sigint).await
    };
    if let Err(e) = repl.engine.save_now() {
        console.err(&format!("  could not save the session: {e}"));
    }
    code
}

/// A provider whose probe timed out at startup (a lazy wrapper's first run) is asked again in
/// the background; the banner above did not wait for it. The runtime learns the answer in
/// [`settle_late_probes`], and the notice is printed through the console, so it never lands in
/// the middle of a turn's output or of the line being typed.
pub(super) fn recheck_slow_providers(repl: &mut Repl, console: &Console) {
    let notices = console.clone();
    repl.engine.runtime.retry_slow_probes(move |a| notices.notify(&format!("{} is ready now", a.provider)));
}

/// Before a prompt is shown or a line is run: apply what the background re-check found and
/// print the notices held back for the prompt. Never mid-turn, so routing cannot change under one.
pub(super) fn settle_late_probes(repl: &mut Repl) {
    repl.engine.runtime.absorb_late();
    repl.console.flush_notices();
}

/// The banner, the offer to resume (or the resume itself), and the one-time hint.
pub(super) fn startup(repl: &mut Repl, args: &Args, term: &Term, console: &Console, state_dir: Option<&Path>, home: Option<&str>) {
    let (style, width, now) = (term.out_style, term.width, log::now_ms());
    let rows = views::providers::avail_rows(&repl.engine.runtime, now);
    console.out(&views::startup::banner(repl.engine.dir(), home, repl.engine.enabled(), &rows, style, width));
    if let Some(notice) = state_dir.and_then(|d| crate::turn::infrastructure::inflight::recovery_notice(d, repl.engine.dir())) {
        console.out(&style.yellow(&notice));
    }
    if repl.engine.hooks().settings_changed {
        console.out(&style.yellow("⚠ this project's .claude/settings.json changed since you trusted it, so Claude Code is not loading it: /hooks to review, /hooks trust to accept"));
    }
    if args.resume {
        match repl.engine.resume() {
            Ok(info) => console.out(&views::startup::resumed_line(&info, now, style)),
            Err(e) => console.out(&style.dim(&format!("no session to continue ({e}); starting fresh"))),
        }
    } else if let Some(info) = state_dir.and_then(|d| session_file::saved_session_info_in(d, repl.engine.dir(), now)) {
        console.out(&views::startup::previous_session_line(&info, now, style));
    }
    if term.interactive() {
        if repl.console.front().is_none() {
            console.out(&uifmt::truncate_to_width(&style.dim("/help commands  ·  Shift+Tab mode"), width));
        }
        if let Some(hint) = views::startup::first_run_hint_once(&repl.engine.runtime.avail, state_dir) {
            for line in hint.lines() {
                console.out(&style.dim(line));
            }
            console.out(&style.dim("Run /doctor for the full check and the exact commands."));
        }
    }
}
