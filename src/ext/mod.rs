//! Extensibility: what the person adds to jev-code without changing it.
//!
//! * [`library`]: custom slash commands, skills and recipes from markdown files;
//! * [`hooks`]: shell commands on tool calls and turn events;
//! * [`market`]: install, update and remove extensions with a checksum for every file
//!   (`jev-code ext ...`);
//! * [`frontmatter`], [`sha256`]: their small parsers.
//!
//! Everything here is pure or works on folders it is given, so each rule is a unit test; the
//! session only maps [`ExtCmd`] and unknown slash words onto it.

pub mod cli;
pub mod frontmatter;
pub mod hooks;
pub mod library;
pub mod market;
pub mod view;

use std::path::{Path, PathBuf};

/// Where extension files are looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub project: PathBuf,
    /// The user's config folder; `None` when there is no home.
    pub user: Option<PathBuf>,
}

/// `$JEV_CODE_CONFIG_DIR`, else `$XDG_CONFIG_HOME/jev-code`, else `~/.config/jev-code`.
pub fn config_dir(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    set("JEV_CODE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| set("XDG_CONFIG_HOME").map(|x| Path::new(&x).join("jev-code")))
        .or_else(|| set("HOME").map(|h| Path::new(&h).join(".config").join("jev-code")))
}

impl Places {
    pub fn from_env(project: &Path) -> Self {
        Self { project: project.to_path_buf(), user: config_dir(|n| std::env::var(n).ok()) }
    }
}

/// The session commands of this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtCmd {
    /// `/extensions`: everything that was found.
    List,
    /// `/recipe`: the recipes.
    Recipes,
    /// `/recipe <name> [arguments]`.
    RunRecipe { name: String, args: String },
    /// `/skill`: the skills, with how to use them.
    Skills,
    /// `/skill <name> [arguments]`: run a skill.
    RunSkill { name: String, args: String },
    /// `/skill show <name>`: read a skill before running it.
    ShowSkill(String),
    /// `/skill new <name> [description]`: start a skill in this project.
    NewSkill { name: String, description: String },
    /// `/hooks`.
    Hooks,
    /// `/settings`: the layered settings and profiles ([`crate::layers`]).
    Settings,
    /// `/hooks trust`: let this project's own hooks run.
    TrustHooks,
}

/// Read `/extensions`, `/recipe` and `/hooks`; `None` for any other word. The error is the usage line.
pub fn parse_command(head: &str, rest: Option<&str>) -> Option<Result<ExtCmd, String>> {
    let rest = rest.map(str::trim).filter(|r| !r.is_empty());
    Some(match (head, rest) {
        ("/extensions", None) => Ok(ExtCmd::List),
        ("/extensions", Some(_)) => Err("usage: /extensions".into()),
        ("/recipe", None) => Ok(ExtCmd::Recipes),
        ("/recipe", Some(text)) => {
            let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
            Ok(ExtCmd::RunRecipe { name: name.to_string(), args: args.trim().to_string() })
        }
        ("/skill", None) | ("/skill", Some("list")) => Ok(ExtCmd::Skills),
        ("/skill", Some(text)) => {
            let (word, tail) = text.split_once(char::is_whitespace).map_or((text, ""), |(w, t)| (w, t.trim()));
            match (word, tail) {
                ("show", name) if library::valid_name(name) => Ok(ExtCmd::ShowSkill(name.to_string())),
                ("show", _) => Err("usage: /skill show <name>".into()),
                ("new", t) => {
                    let (name, description) = t.split_once(char::is_whitespace).map_or((t, ""), |(n, d)| (n, d.trim()));
                    if library::valid_name(name) && !library::SKILL_WORDS.contains(&name) {
                        Ok(ExtCmd::NewSkill { name: name.to_string(), description: description.to_string() })
                    } else {
                        Err("usage: /skill new <name> [what it is for] (name: lowercase letters, digits, - and _; not list, show or new)".into())
                    }
                }
                (name, args) => Ok(ExtCmd::RunSkill { name: name.to_string(), args: args.to_string() }),
            }
        }
        ("/settings", None) => Ok(ExtCmd::Settings),
        ("/settings", Some(_)) => Err("usage: /settings".into()),
        ("/hooks", None) => Ok(ExtCmd::Hooks),
        ("/hooks", Some("trust")) => Ok(ExtCmd::TrustHooks),
        ("/hooks", Some(_)) => Err("usage: /hooks [trust]".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_prefers_the_override_then_xdg_then_home() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |n: &str| pairs.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        assert_eq!(config_dir(env(&[("JEV_CODE_CONFIG_DIR", "/x"), ("HOME", "/h")])), Some(PathBuf::from("/x")));
        assert_eq!(config_dir(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")])), Some(PathBuf::from("/xdg/jev-code")));
        assert_eq!(config_dir(env(&[("HOME", "/h")])), Some(PathBuf::from("/h/.config/jev-code")));
        assert_eq!(config_dir(env(&[("HOME", "")])), None);
    }

    #[test]
    fn commands_parse_with_their_arguments() {
        assert_eq!(parse_command("/extensions", None), Some(Ok(ExtCmd::List)));
        assert_eq!(parse_command("/recipe", None), Some(Ok(ExtCmd::Recipes)));
        assert_eq!(parse_command("/skill", None), Some(Ok(ExtCmd::Skills)));
        assert_eq!(parse_command("/skill", Some("list")), Some(Ok(ExtCmd::Skills)));
        assert_eq!(parse_command("/skill", Some("show db")), Some(Ok(ExtCmd::ShowSkill("db".into()))));
        assert!(matches!(parse_command("/skill", Some("show")), Some(Err(_))));
        assert_eq!(
            parse_command("/skill", Some("new db  query the db ")),
            Some(Ok(ExtCmd::NewSkill { name: "db".into(), description: "query the db".into() }))
        );
        assert!(matches!(parse_command("/skill", Some("new")), Some(Err(_))));
        assert!(matches!(parse_command("/skill", Some("new list")), Some(Err(_))));
        assert_eq!(
            parse_command("/skill", Some("db the users table")),
            Some(Ok(ExtCmd::RunSkill { name: "db".into(), args: "the users table".into() }))
        );
        assert_eq!(
            parse_command("/recipe", Some(" fix-tests  the parser ")),
            Some(Ok(ExtCmd::RunRecipe { name: "fix-tests".into(), args: "the parser".into() }))
        );
        assert_eq!(parse_command("/hooks", Some("trust")), Some(Ok(ExtCmd::TrustHooks)));
        assert_eq!(parse_command("/hooks", Some("x")), Some(Err("usage: /hooks [trust]".into())));
        assert_eq!(parse_command("/model", None), None);
    }
}
