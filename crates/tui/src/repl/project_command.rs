//! `/project`: what kind of project this is, its map, its checks (with an optional
//! fix-and-rerun loop) and the notes every session of it starts with. Parsing only.

use jev_orchestration::infrastructure::project_kind::Check;

/// What `/project` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectCmd {
    /// Kind, commands and notes at a glance.
    Overview,
    /// The repository map, narrowed by words.
    Map(Vec<String>),
    /// Run a check; with `fix`, hand failures to the model and rerun.
    Check { check: Check, fix: bool },
    Note(String),
    Notes,
    Forget(usize),
}

const USAGE: &str = "usage: /project [map [words] | check [test|lint|build] [fix] | note <text> | notes | forget <number>]";

pub fn parse(args: &str) -> Result<ProjectCmd, String> {
    let args = args.trim();
    let (verb, rest) = args.split_once(char::is_whitespace).map_or((args, ""), |(v, r)| (v, r.trim()));
    match verb {
        "" => Ok(ProjectCmd::Overview),
        "map" => Ok(ProjectCmd::Map(rest.split_whitespace().map(str::to_string).collect())),
        "check" => parse_check(rest),
        "note" if rest.is_empty() => Err("write the note after `note`".into()),
        "note" => Ok(ProjectCmd::Note(rest.to_string())),
        "notes" => Ok(ProjectCmd::Notes),
        "forget" => rest.parse::<usize>().ok().filter(|n| *n > 0).map(ProjectCmd::Forget).ok_or_else(|| "forget needs a note number from /project notes".to_string()),
        _ => Err(USAGE.into()),
    }
}

fn parse_check(rest: &str) -> Result<ProjectCmd, String> {
    let mut check = None;
    let mut fix = false;
    for word in rest.split_whitespace() {
        match (word, Check::parse(word)) {
            ("fix", _) => fix = true,
            (_, Some(c)) if check.is_none() => check = Some(c),
            _ => return Err(format!("`{word}` is not a check; {USAGE}")),
        }
    }
    Ok(ProjectCmd::Check { check: check.unwrap_or(Check::Test), fix })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbs_and_their_arguments() {
        assert_eq!(parse(""), Ok(ProjectCmd::Overview));
        assert_eq!(parse("map router  ui"), Ok(ProjectCmd::Map(vec!["router".into(), "ui".into()])));
        assert_eq!(parse("check"), Ok(ProjectCmd::Check { check: Check::Test, fix: false }));
        assert_eq!(parse("check lint fix"), Ok(ProjectCmd::Check { check: Check::Lint, fix: true }));
        assert_eq!(parse("check fix"), Ok(ProjectCmd::Check { check: Check::Test, fix: true }));
        assert_eq!(parse("note use tabs"), Ok(ProjectCmd::Note("use tabs".into())));
        assert_eq!(parse("forget 2"), Ok(ProjectCmd::Forget(2)));
    }

    #[test]
    fn mistakes_explain_themselves() {
        for bad in ["note", "forget", "forget 0", "forget x", "check deploy", "check test lint", "wat"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("wat").unwrap_err().starts_with("usage:"));
    }
}
