//! Owns `jev-code`'s `run`: argument handling, terminal setup and the choice of mode. Every exit
//! goes through its return value, so the terminal guard always runs.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use crate::tui::cli_args as cli;
use crate::tui::cli_args::{Action, Args};
use crate::tui::console::front::Front;
use crate::shared::infrastructure::jsonl_log as log;
use crate::tui::tty::terminal_guard as terminal;
use crate::tui::tty::terminal_guard::TerminalGuard;
use crate::tui::render as uifmt;
use crate::tui::render::Style;
use super::oneshot::{doctor, print_mode};
use super::session::session;
use super::shell_commands::{ext_command, update_command};
use super::term::{console_for, Term};

pub async fn run() -> i32 {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("jev-code: {e}\n{}", cli::USAGE);
            return 2;
        }
    };
    match args.action {
        Action::Help => {
            println!("{}", cli::HELP);
            return 0;
        }
        Action::Version => {
            println!("jev-code {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        Action::Ext(action) => return ext_command(&action, args.no_color),
        Action::Update { yes } => return update_command(yes),
        _ => {}
    }
    let term = Term::detect(args.no_color);
    let _terminal = TerminalGuard::install();
    let state_dir = log::state_dir();
    terminal::install_panic_hook(state_dir.as_ref().map(|d| d.join("panic.log")));
    terminal::restore_on_termination();

    let dir = match project_dir(&args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    apply_layered_settings(&args, &dir);
    // A person at a terminal gets the interactive screen; everything else prints plainly.
    let (front, front_rx) = match args.action {
        Action::Doctor | Action::Print(_) => (None, None),
        _ if args.startup_profile => (None, None),
        _ if term.interactive() && std::env::var_os("JEV_CLASSIC_UI").is_none() => {
            let (front, rx) = Front::channel();
            (Some(front), Some(rx))
        }
        _ => (None, None),
    };
    let console = console_for(&term, &dir, !matches!(args.action, Action::Print(_)), front);
    match &args.action {
        Action::Doctor => doctor(&dir, &console).await,
        Action::Print(source) => print_mode(&args, source, &dir, &term, console, state_dir).await,
        _ => session(&args, &dir, &term, console, state_dir, front_rx).await,
    }
}

/// Settings from the config files and the chosen profile reach the process environment
/// before anything reads one (`/settings` shows them). Problems are shown there, not here.
pub(super) fn apply_layered_settings(args: &Args, dir: &Path) {
    if let Some(profile) = &args.profile {
        std::env::set_var("JEV_PROFILE", profile);
    }
    let profile = std::env::var("JEV_PROFILE").ok().filter(|p| !p.is_empty());
    let places = crate::approval::application::extensions::Places::from_env(dir);
    crate::tui::app::layered_settings::Layered::load(&places, profile.as_deref()).apply_to_process();
}

/// The project directory, or the message that says why it cannot be used.
pub(super) fn project_dir(args: &Args) -> Result<PathBuf, String> {
    let given = match &args.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir().map_err(|e| format!("jev-code: cannot tell the current directory: {e}"))?,
    };
    given.canonicalize().map_err(|e| {
        let style = Style::detect(std::io::stderr().is_terminal());
        let steps = ["Check the path, or start jev-code without one to use the current directory".to_string()];
        uifmt::problem_block("Can't open the project folder", &[format!("{}: {e}", given.display())], &steps, style)
    })
}
