//! The trust, git, team and insight commands (`/commit`, `/pr`, `/profile`, `/team`, `/audit`,
//! `/history`, `/bench`, `/replay`): what each line means. Kept apart from
//! [`crate::tui::command`] so that file only gains one variant; the names, help text and
//! completion are in the command table in [`crate::tui::render`], like every other command.

use crate::approval::domain::profile::Profile;
use crate::tui::render as uifmt;

/// How many audit entries `/audit` shows when no number is given.
pub const AUDIT_DEFAULT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtCmd {
    /// `/commit [hint]`.
    Commit(Option<String>),
    /// `/pr [base]`.
    Pr(Option<String>),
    /// `/profile [name]`: no name shows the profiles.
    Profile(Option<Profile>),
    /// `/team`: the repo's shared rules.
    Team,
    /// `/audit [n]`.
    Audit(usize),
    /// `/history`: usage per day.
    History,
    /// `/bench`: which model served best.
    Bench,
    /// `/replay [n]`: read a saved session (the n-th of `/resume`'s list, newest is 1).
    Replay(usize),
    /// A command word with an argument it does not take, or one it cannot read.
    Invalid(String),
}

fn usage(name: &str) -> ExtCmd {
    ExtCmd::Invalid(format!("usage: {}", uifmt::command_usage(name).unwrap_or_else(|| name.to_string())))
}

/// A positive number, or `default` when there is no argument; `None` for anything else.
fn count(arg: Option<&str>, default: usize) -> Option<usize> {
    match arg {
        None => Some(default),
        Some(a) => a.parse().ok().filter(|n| *n > 0),
    }
}

/// `head` is the lowercased command word, `rest` the text after it. `None` when `head` is not
/// one of these commands.
pub fn parse(head: &str, rest: &str) -> Option<ExtCmd> {
    let rest = rest.trim();
    let free = (!rest.is_empty()).then(|| rest.to_string());
    let mut words = rest.split_whitespace();
    let (arg, extra) = (words.next(), words.next().is_some());
    Some(match head {
        "/commit" => ExtCmd::Commit(free),
        "/pr" => match (arg, extra) {
            (_, true) => usage(head),
            _ => ExtCmd::Pr(free),
        },
        "/profile" => match (arg, extra) {
            (None, _) => ExtCmd::Profile(None),
            (Some(name), false) => name.parse().map_or_else(|e: crate::approval::domain::profile::UnknownProfile| ExtCmd::Invalid(e.to_string()), |p| ExtCmd::Profile(Some(p))),
            _ => usage(head),
        },
        "/team" | "/history" | "/bench" if arg.is_some() => usage(head),
        "/team" => ExtCmd::Team,
        "/history" => ExtCmd::History,
        "/bench" => ExtCmd::Bench,
        "/audit" | "/replay" if extra => usage(head),
        "/audit" => count(arg, AUDIT_DEFAULT).map_or_else(|| usage(head), ExtCmd::Audit),
        "/replay" => count(arg, 1).map_or_else(|| usage(head), ExtCmd::Replay),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::command::{parse as parse_line, Command};

    fn ext(line: &str) -> ExtCmd {
        match parse_line(line) {
            Command::Trust(e) => e,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn git_commands_take_free_text() {
        assert_eq!(ext("/commit"), ExtCmd::Commit(None));
        assert_eq!(ext("/commit the parser fix, split in two"), ExtCmd::Commit(Some("the parser fix, split in two".into())));
        assert_eq!(ext("/pr develop"), ExtCmd::Pr(Some("develop".into())));
        assert_eq!(ext("/pr"), ExtCmd::Pr(None));
        assert!(matches!(ext("/pr a b"), ExtCmd::Invalid(m) if m.starts_with("usage: /pr")));
    }

    #[test]
    fn profile_names_are_checked() {
        assert_eq!(ext("/profile"), ExtCmd::Profile(None));
        assert_eq!(ext("/PROFILE Careful"), ExtCmd::Profile(Some(Profile::Careful)));
        assert!(matches!(ext("/profile yolo"), ExtCmd::Invalid(m) if m.contains("not a profile")));
        assert!(matches!(ext("/profile a b"), ExtCmd::Invalid(_)));
    }

    #[test]
    fn insight_commands_take_a_count_or_nothing() {
        assert_eq!(ext("/audit"), ExtCmd::Audit(AUDIT_DEFAULT));
        assert_eq!(ext("/audit 5"), ExtCmd::Audit(5));
        assert!(matches!(ext("/audit 0"), ExtCmd::Invalid(_)));
        assert!(matches!(ext("/audit lots"), ExtCmd::Invalid(_)));
        assert_eq!(ext("/replay"), ExtCmd::Replay(1));
        assert_eq!(ext("/replay 3"), ExtCmd::Replay(3));
        assert_eq!(ext("/team"), ExtCmd::Team);
        assert_eq!(ext("/history"), ExtCmd::History);
        assert_eq!(ext("/bench"), ExtCmd::Bench);
        assert_eq!(ext("/history now"), ExtCmd::Invalid("usage: /history".into()));
    }

    #[test]
    fn other_words_are_not_ours() {
        assert_eq!(parse("/model", ""), None);
    }
}
