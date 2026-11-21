//! The arguments of `/diff` and the checkpoint list `/diff --list` and `/rewind` print:
//! parsing and wording on the interface side, the scope itself is [`Scope`].

use crate::changes::application::diff_command::Scope;
use crate::changes::domain::change_set::ChangeSummary;
use crate::changes::domain::file_change::FileChange;
use crate::tui::render::{Fold, Layout, Style};

/// `/diff [--turn N | --all | --list] [--split | --unified] [--full | --collapse]
/// [--review] [--export FILE] [file]`, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffArgs {
    pub scope: Scope,
    pub path: Option<String>,
    pub layout: Layout,
    /// `None` is the default fold (long files are cut).
    pub fold: Option<Fold>,
    pub review: bool,
    pub list: bool,
    pub export: Option<String>,
}

impl Default for DiffArgs {
    fn default() -> Self {
        Self { scope: Scope::Last, path: None, layout: Layout::Auto, fold: None, review: false, list: false, export: None }
    }
}

pub const USAGE: &str = "usage: /diff [--turn N | --all | --list] [--split | --unified] [--full | --collapse] [--review] [--export FILE] [file]";

/// Parse the text after `/diff`. Words that do not start with `--` make the file path
/// (joined, so a name with spaces works), and a path may follow flags in any order.
pub fn parse_args(rest: &str) -> Result<DiffArgs, String> {
    let mut args = DiffArgs::default();
    let mut path: Vec<&str> = Vec::new();
    let mut words = rest.split_whitespace();
    while let Some(w) = words.next() {
        match w {
            "--all" => args.scope = Scope::All,
            "--list" => args.list = true,
            "--split" | "--side-by-side" => args.layout = Layout::Split,
            "--unified" => args.layout = Layout::Unified,
            "--full" => args.fold = Some(Fold::Open),
            "--collapse" => args.fold = Some(Fold::Collapsed),
            "--review" => args.review = true,
            "--turn" => {
                let n = words.next().and_then(|n| n.parse::<usize>().ok()).filter(|n| *n > 0).ok_or(USAGE)?;
                args.scope = Scope::Turn(n);
            }
            "--export" => args.export = Some(words.next().ok_or(USAGE)?.to_string()),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}; {USAGE}")),
            _ => path.push(w),
        }
    }
    if !path.is_empty() {
        args.path = Some(path.join(" "));
    }
    Ok(args)
}

/// One row per checkpoint: its number, the files it touched and the line counts.
pub fn checkpoint_list(history: &[ChangeSummary], style: Style) -> String {
    if history.is_empty() {
        return style.dim("  No turn of this session has changed files yet.");
    }
    let rows: Vec<String> = history
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (a, r) = s.totals();
            let names: Vec<&str> = s.files.iter().map(|f: &FileChange| f.path.as_str()).collect();
            let shown = if names.len() > 3 { format!("{} and {} more", names[..3].join(", "), names.len() - 3) } else { names.join(", ") };
            format!("  {} {}  {} {}  {}", style.bold(&format!("{:>2}", i + 1)), shown, style.green(&format!("+{a}")), style.red(&format!("−{r}")), style.dim(&format!("{} file(s)", s.files.len())))
        })
        .collect();
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::domain::file_change::ChangeStatus;

    fn summary(before: &str, after: &str, file: &str) -> ChangeSummary {
        let f = FileChange { path: file.into(), status: ChangeStatus::Modified, added: 2, removed: 1, old_path: None };
        ChangeSummary { files: vec![f], before: before.into(), after: after.into(), ..ChangeSummary::default() }
    }

    #[test]
    fn arguments_mix_flags_and_a_path_with_spaces() {
        assert_eq!(parse_args("").unwrap(), DiffArgs::default());
        let a = parse_args("--turn 2 --split my notes/a b.md --export out.patch").unwrap();
        assert_eq!((a.scope, a.layout, a.path.as_deref(), a.export.as_deref()), (Scope::Turn(2), Layout::Split, Some("my notes/a b.md"), Some("out.patch")));
        let b = parse_args("--all --full --review").unwrap();
        assert_eq!((b.scope, b.fold, b.review), (Scope::All, Some(Fold::Open), true));
        assert_eq!(parse_args("--collapse --list").unwrap().fold, Some(Fold::Collapsed));
    }

    #[test]
    fn bad_arguments_say_what_is_accepted() {
        for bad in ["--turn", "--turn 0", "--turn x", "--export", "--bogus"] {
            let e = parse_args(bad).unwrap_err();
            assert!(e.contains("usage: /diff"), "{bad}: {e}");
        }
    }

    #[test]
    fn the_checkpoint_list_numbers_turns() {
        let h = [summary("t0", "t1", "a.rs"), summary("t1", "t2", "b.rs")];
        let out = checkpoint_list(&h, Style::off());
        assert_eq!(out.lines().next().unwrap(), "   1 a.rs  +2 −1  1 file(s)");
        assert!(out.lines().nth(1).unwrap().starts_with("   2 b.rs"));
        assert!(checkpoint_list(&[], Style::off()).contains("No turn"));
    }
}
