//! `jev-code ext ...`: install, update, remove, list and pack extensions from the shell.
//!
//!   jev-code ext list
//!   jev-code ext install <folder>
//!   jev-code ext update [name]
//!   jev-code ext remove <name>
//!   jev-code ext pack <folder> <name> <version>
//!
//! [`parse`] and [`run`] are pure over a config folder, so each spelling and outcome is a test.

use std::path::{Path, PathBuf};

use super::{market, view};
use crate::uifmt::Style;

pub const USAGE: &str = "usage: jev-code ext list\n       jev-code ext install <folder>\n       jev-code ext update [name]\n       jev-code ext remove <name>\n       jev-code ext pack <folder> <name> <version>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtAction {
    List,
    Install(PathBuf),
    /// `None`: every installed extension.
    Update(Option<String>),
    Remove(String),
    Pack { dir: PathBuf, name: String, version: String },
}

/// Read what follows `ext`. The error is the usage text.
pub fn parse(args: &[String]) -> Result<ExtAction, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["list"] => Ok(ExtAction::List),
        ["install", folder] => Ok(ExtAction::Install(PathBuf::from(folder))),
        ["update"] => Ok(ExtAction::Update(None)),
        ["update", name] => Ok(ExtAction::Update(Some((*name).to_string()))),
        ["remove", name] => Ok(ExtAction::Remove((*name).to_string())),
        ["pack", dir, name, version] => Ok(ExtAction::Pack { dir: PathBuf::from(dir), name: (*name).to_string(), version: (*version).to_string() }),
        _ => Err(USAGE.to_string()),
    }
}

/// What a run printed and the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    pub code: i32,
}

fn ok(text: String) -> Outcome {
    Outcome { text, code: 0 }
}

fn failed(style: Style, e: &market::MarketError) -> Outcome {
    Outcome { text: format!("{} {e}", style.red("✗")), code: 1 }
}

fn installed_line(done: &market::Installed, style: Style) -> String {
    let replaced = done.replaced.as_ref().map(|v| format!(" (was {v})")).unwrap_or_default();
    format!("{} {} {} installed, {} file(s) verified{replaced}", style.green("✓"), done.name, done.version, done.files)
}

/// Do the action against the user's config folder.
pub fn run(action: &ExtAction, user: &Path, style: Style) -> Outcome {
    match action {
        ExtAction::List => ok(view::installed_text(&market::list(user), style)),
        ExtAction::Install(folder) => market::install(user, folder).map_or_else(|e| failed(style, &e), |done| ok(installed_line(&done, style))),
        ExtAction::Update(name) => update(user, name.as_deref(), style),
        ExtAction::Remove(name) => market::remove(user, name).map_or_else(|e| failed(style, &e), |()| ok(format!("{} {name} removed", style.green("✓")))),
        ExtAction::Pack { dir, name, version } => market::pack(dir, name, version)
            .map_or_else(|e| failed(style, &e), |m| ok(format!("{} wrote {} for {} {} ({} file(s))", style.green("✓"), market::MANIFEST, m.name, m.version, m.files.len()))),
    }
}

fn update(user: &Path, name: Option<&str>, style: Style) -> Outcome {
    let names: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => market::list(user).into_iter().map(|l| l.name).collect(),
    };
    if names.is_empty() {
        return ok(style.dim("no extensions installed"));
    }
    let mut lines = Vec::new();
    let mut code = 0;
    for n in names {
        match market::update(user, &n) {
            Ok(Some(done)) => lines.push(installed_line(&done, style)),
            Ok(None) => lines.push(format!("{} {n} is up to date", style.dim("·"))),
            Err(e) => {
                code = 1;
                lines.push(failed(style, &e).text);
            }
        }
    }
    Outcome { text: lines.join("\n"), code }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jev-extcli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_spelling_parses_or_gives_the_usage() {
        assert_eq!(parse(&words("list")), Ok(ExtAction::List));
        assert_eq!(parse(&words("install ./kit")), Ok(ExtAction::Install(PathBuf::from("./kit"))));
        assert_eq!(parse(&words("update")), Ok(ExtAction::Update(None)));
        assert_eq!(parse(&words("update kit")), Ok(ExtAction::Update(Some("kit".into()))));
        assert_eq!(parse(&words("remove kit")), Ok(ExtAction::Remove("kit".into())));
        assert_eq!(parse(&words("pack ./kit kit 1.0.0")), Ok(ExtAction::Pack { dir: "./kit".into(), name: "kit".into(), version: "1.0.0".into() }));
        assert_eq!(parse(&words("install")), Err(USAGE.to_string()));
        assert_eq!(parse(&[]), Err(USAGE.to_string()));
    }

    #[test]
    fn pack_install_update_and_remove_report_what_happened() {
        let root = temp("flow");
        let (user, src) = (root.join("user"), root.join("src"));
        std::fs::create_dir_all(src.join("commands")).unwrap();
        std::fs::write(src.join("commands/hi.md"), "say hi").unwrap();
        let off = Style::off();
        let packed = run(&ExtAction::Pack { dir: src.clone(), name: "kit".into(), version: "1.0.0".into() }, &user, off);
        assert_eq!(packed.code, 0, "{}", packed.text);
        let installed = run(&ExtAction::Install(src.clone()), &user, off);
        assert!(installed.text.contains("kit 1.0.0 installed, 1 file(s) verified"), "{}", installed.text);
        assert_eq!(run(&ExtAction::Update(None), &user, off).text, "· kit is up to date");
        assert!(run(&ExtAction::List, &user, off).text.contains("kit 1.0.0  verified"));
        let removed = run(&ExtAction::Remove("kit".into()), &user, off);
        assert_eq!((removed.code, removed.text.as_str()), (0, "✓ kit removed"));
        let again = run(&ExtAction::Remove("kit".into()), &user, off);
        assert_eq!((again.code, again.text.as_str()), (1, "✗ kit is not installed"));
    }

    #[test]
    fn a_tampered_source_is_refused_with_a_nonzero_exit() {
        let root = temp("tamper");
        let (user, src) = (root.join("user"), root.join("src"));
        std::fs::create_dir_all(src.join("commands")).unwrap();
        std::fs::write(src.join("commands/hi.md"), "say hi").unwrap();
        run(&ExtAction::Pack { dir: src.clone(), name: "kit".into(), version: "1.0.0".into() }, &user, Style::off());
        std::fs::write(src.join("commands/hi.md"), "say something else").unwrap();
        let out = run(&ExtAction::Install(src), &user, Style::off());
        assert_eq!(out.code, 1);
        assert!(out.text.contains("checksum mismatch"), "{}", out.text);
    }
}
