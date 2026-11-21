//! How `/sessions` reads its words. Pure, so every spelling is a unit test.

use crate::session::infrastructure::session_import::Source;
use crate::session::application::session_library::marks::Mark;
use crate::session::application::session_library::query::Query;
use crate::session::infrastructure::session_meta::clean_tag;

/// Days `/sessions clean` keeps when none is given.
pub const DEFAULT_CLEAN_DAYS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionsCmd {
    /// A listing over all projects; with words and filters it is a search.
    Find(Query),
    /// A change to marks of session `n` of the last listing.
    Mark { n: usize, mark: Mark },
    Export { n: usize, file: Option<String> },
    /// A file made by `export`.
    ImportFile(String),
    /// Histories of this folder in Claude Code or Codex; with `pick` the n-th is imported.
    ImportHistory { source: Source, pick: Option<usize> },
    Disk,
    Clean { days: u64 },
}

pub const USAGE: &str = "/sessions [words] [provider:x tag:x file:x since:7d project:x pinned|archived|all] | pin|unpin|archive|unarchive <n> | tag|untag <n> <tag> | export <n> [file] | import <file>|claude|codex [n] | disk | clean [days]";

fn number(word: Option<&str>) -> Result<usize, String> {
    word.and_then(|w| w.parse::<usize>().ok()).filter(|n| *n > 0).ok_or_else(|| format!("give the number from the list, e.g. /sessions pin 2\nusage: {USAGE}"))
}

/// The arguments after `/sessions`.
pub fn parse(args: &str) -> Result<SessionsCmd, String> {
    let words: Vec<&str> = args.split_whitespace().collect();
    let rest = |from: usize| words.iter().skip(from).copied().collect::<Vec<_>>();
    match words.first().copied() {
        Some("pin") => Ok(SessionsCmd::Mark { n: number(words.get(1).copied())?, mark: Mark::Pin(true) }),
        Some("unpin") => Ok(SessionsCmd::Mark { n: number(words.get(1).copied())?, mark: Mark::Pin(false) }),
        Some("archive") => Ok(SessionsCmd::Mark { n: number(words.get(1).copied())?, mark: Mark::Archive(true) }),
        Some("unarchive") => Ok(SessionsCmd::Mark { n: number(words.get(1).copied())?, mark: Mark::Archive(false) }),
        Some(verb @ ("tag" | "untag")) => {
            let n = number(words.get(1).copied())?;
            let tag = words.get(2).and_then(|t| clean_tag(t)).ok_or("a tag is one word of letters, digits, - _ . (e.g. /sessions tag 2 bug)")?;
            Ok(SessionsCmd::Mark { n, mark: if verb == "tag" { Mark::Tag(tag) } else { Mark::Untag(tag) } })
        }
        Some("export") => Ok(SessionsCmd::Export { n: number(words.get(1).copied())?, file: (words.len() > 2).then(|| rest(2).join(" ")) }),
        Some("import") => match (words.get(1).copied(), words.get(2).copied()) {
            (None, _) => Err("import what? a file made by /sessions export, or claude, or codex".into()),
            (Some(source @ ("claude" | "codex")), pick) => Ok(SessionsCmd::ImportHistory {
                source: if source == "claude" { Source::ClaudeCode } else { Source::Codex },
                pick: pick.map(|p| number(Some(p))).transpose()?,
            }),
            (Some(_), _) => Ok(SessionsCmd::ImportFile(rest(1).join(" "))),
        },
        Some("disk" | "usage") if words.len() == 1 => Ok(SessionsCmd::Disk),
        Some("clean") => {
            let days = match words.get(1) {
                None => DEFAULT_CLEAN_DAYS,
                Some(d) => d.parse::<u64>().ok().filter(|d| *d > 0).ok_or("give the age in days, e.g. /sessions clean 30")?,
            };
            Ok(SessionsCmd::Clean { days })
        }
        Some("find") => Query::parse(&rest(1).join(" ")).map(SessionsCmd::Find),
        _ => Query::parse(args).map(SessionsCmd::Find),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_filters_are_a_search_and_verbs_are_actions() {
        assert_eq!(parse("").unwrap(), SessionsCmd::Find(Query::default()));
        assert!(matches!(parse("parser tag:bug"), Ok(SessionsCmd::Find(q)) if q.words == ["parser"] && q.tag.as_deref() == Some("bug")));
        assert!(matches!(parse("find pin"), Ok(SessionsCmd::Find(q)) if q.words == ["pin"]));
        assert_eq!(parse("pin 2").unwrap(), SessionsCmd::Mark { n: 2, mark: Mark::Pin(true) });
        assert_eq!(parse("unarchive 1").unwrap(), SessionsCmd::Mark { n: 1, mark: Mark::Archive(false) });
        assert_eq!(parse("tag 3 #Bug").unwrap(), SessionsCmd::Mark { n: 3, mark: Mark::Tag("bug".into()) });
        assert_eq!(parse("export 1 my notes.json").unwrap(), SessionsCmd::Export { n: 1, file: Some("my notes.json".into()) });
        assert_eq!(parse("export 1").unwrap(), SessionsCmd::Export { n: 1, file: None });
        assert_eq!(parse("import a b.json").unwrap(), SessionsCmd::ImportFile("a b.json".into()));
        assert_eq!(parse("import codex 2").unwrap(), SessionsCmd::ImportHistory { source: Source::Codex, pick: Some(2) });
        assert_eq!(parse("import claude").unwrap(), SessionsCmd::ImportHistory { source: Source::ClaudeCode, pick: None });
        assert_eq!(parse("disk").unwrap(), SessionsCmd::Disk);
        assert_eq!(parse("clean").unwrap(), SessionsCmd::Clean { days: DEFAULT_CLEAN_DAYS });
        assert_eq!(parse("clean 7").unwrap(), SessionsCmd::Clean { days: 7 });
    }

    #[test]
    fn mistakes_say_what_is_wanted() {
        for bad in ["pin", "pin x", "pin 0", "tag 1", "tag 1 bad!tag","import", "clean 0", "clean soon", "provider:nobody", "export"] {
            let e = parse(bad).unwrap_err();
            assert!(!e.is_empty(), "{bad}");
        }
        assert!(parse("pin").unwrap_err().contains("usage:"));
    }
}
