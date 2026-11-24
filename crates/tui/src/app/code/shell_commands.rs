//! Owns the two commands that need no providers and no terminal setup: `update` and `ext`.

use std::io::IsTerminal;
use crate::render::Style;

/// `jev-code update [--yes]`: the same as `/update`, from the shell. Asks on the terminal unless
/// `--yes`; without a terminal to ask on, only `--yes` updates. Exit code: 0 updated or already
/// the newest, 1 refused or failed, 2 not confirmed.
pub(super) fn update_command(yes: bool) -> i32 {
    use crate::app::self_update as selfupdate;
    use crate::app::self_update::Prepared;
    use std::io::{BufRead, IsTerminal, Write};
    let source = match selfupdate::source() {
        Ok(source) => source,
        Err(why) => {
            eprintln!("cannot update: {why}");
            return 1;
        }
    };
    eprintln!("checking {} for a new version...", source.label(|d| d.display().to_string()));
    let (lines, question, action) = match selfupdate::prepare(&source) {
        Prepared::UpToDate(lines) => {
            lines.iter().for_each(|l| println!("{l}"));
            return 0;
        }
        Prepared::Refused(lines) => {
            lines.iter().for_each(|l| println!("{l}"));
            return 1;
        }
        Prepared::Offer { lines, question, action } => (lines, question, action),
    };
    lines.iter().for_each(|l| println!("{l}"));
    if !yes {
        if !std::io::stdin().is_terminal() {
            eprintln!("not updated: run it in a terminal to be asked, or pass --yes");
            return 2;
        }
        eprint!("{question}");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            eprintln!("left as it is");
            return 2;
        }
    }
    match action.run(&mut |line| println!("  {line}")) {
        Ok(version) => {
            println!("jev-code {version} is installed");
            0
        }
        Err(why) => {
            eprintln!("{why}");
            1
        }
    }
}

/// `jev-code ext ...`: no providers and no terminal setup, only the config folder.
pub(super) fn ext_command(action: &crate::app::extension_cli::ExtAction, no_color: bool) -> i32 {
    let style = if no_color { Style::off() } else { Style::detect(std::io::stdout().is_terminal()) };
    let Some(user) = jev_approval::application::extensions::config_dir(|n| std::env::var(n).ok()) else {
        eprintln!("jev-code: no config folder for extensions (HOME is not set; or set JEV_CODE_CONFIG_DIR)");
        return 1;
    };
    let outcome = crate::app::extension_cli::run(action, &user, style);
    println!("{}", outcome.text);
    outcome.code
}
