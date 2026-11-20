//! `/permissions` argument parsing: command-line syntax, not approval policy.

use crate::approval::domain::permission_rule::validate_rule;

/// What `/permissions` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionsCmd {
    List,
    Allow(String),
    Remove(String),
    Dir(String),
}

pub const USAGE: &str = "usage: /permissions [allow <rule> | remove <rule> | dir <path>]";

/// The text after `/permissions`. A rule has spaces in it (`Bash(cargo test:*)`), so the
/// rest of the line is the rule.
pub fn parse_args(rest: &str) -> Result<PermissionsCmd, String> {
    let rest = rest.trim();
    let (head, arg) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(h, a)| (h, a.trim()));
    match (head.to_ascii_lowercase().as_str(), arg) {
        ("", _) | ("list", "") => Ok(PermissionsCmd::List),
        ("allow" | "add", rule) if !rule.is_empty() => validate_rule(rule).map(PermissionsCmd::Allow),
        ("remove" | "rm" | "delete", rule) if !rule.is_empty() => Ok(PermissionsCmd::Remove(rule.to_string())),
        ("dir", path) if !path.is_empty() => Ok(PermissionsCmd::Dir(path.to_string())),
        ("allow" | "add", "") => Err(format!("{USAGE}. A rule is a tool name, optionally followed by a pattern in parentheses. Examples: Bash(cargo test:*), Read(//tmp/**), WebSearch")),
        ("remove" | "rm" | "delete" | "dir" | "list", _) => Err(USAGE.to_string()),
        _ => Err(format!("unknown permissions subcommand `{head}`; {USAGE}. Example: /permissions allow Bash(cargo test:*)")),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_parsed() {
        assert_eq!(parse_args(""), Ok(PermissionsCmd::List));
        assert_eq!(parse_args(" list "), Ok(PermissionsCmd::List));
        assert_eq!(parse_args("allow Bash(cargo test:*)"), Ok(PermissionsCmd::Allow("Bash(cargo test:*)".into())));
        assert_eq!(parse_args("ALLOW   Read(//tmp/**) "), Ok(PermissionsCmd::Allow("Read(//tmp/**)".into())));
        assert_eq!(parse_args("remove Bash(cargo test:*)"), Ok(PermissionsCmd::Remove("Bash(cargo test:*)".into())));
        assert_eq!(parse_args("dir ../other dir"), Ok(PermissionsCmd::Dir("../other dir".into())));
        for bad in ["allow", "remove", "dir", "nope", "list extra", "allow Bash("] {
            assert!(parse_args(bad).is_err(), "{bad}");
        }
        assert!(parse_args("nope").unwrap_err().contains("unknown permissions subcommand `nope`"));
        let help = parse_args("allow").unwrap_err();
        assert!(help.contains("Bash(cargo test:*)") && help.contains("Read(//tmp/**)") && help.contains("WebSearch"));
    }
}
