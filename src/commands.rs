//! What one line typed into `jev-code` means. Pure, so the REPL binary only maps
//! each [`Command`] onto the [`crate::turn::Engine`] and every spelling rule is a
//! unit test. The names come from the one command table in [`crate::uifmt`], which also
//! drives `/help` and Tab completion.

use crate::mode::Mode;
use crate::permissions::{self, PermissionsCmd};
use crate::routing::Pin;
use crate::uifmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpCmd {
    List,
    Login(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A blank line.
    Empty,
    /// Anything that is not a command: sent to the model.
    Prompt(String),
    Quit,
    New,
    /// `/model` with no argument: show the current pin.
    ShowModel,
    /// `/model <arg>`.
    SetPin(Pin),
    /// `/provider auto|off`: routing across providers on (true) or off (Claude only).
    SetRouting(bool),
    /// `/orchestrate on|off`: a planner that delegates pieces to workers, for this session.
    SetOrchestrate(bool),
    /// `/orchestrate` with no argument: show whether it is on.
    ShowOrchestrate,
    /// `/mode` with no argument: show the mode.
    ShowMode,
    /// `/mode <name>`.
    SetMode(Mode),
    /// `/providers [refresh]`.
    Providers { refresh: bool },
    /// `/mcp [list | login <server>]`.
    Mcp(McpCmd),
    /// A command that was recognised but whose argument was not; the text says what is accepted.
    Invalid(String),
    /// `/help`.
    Help,
    /// `/status`.
    Status,
    /// `/doctor`: the setup check, inside the session.
    Doctor,
    /// `/diff [file]`: what the last turn changed.
    Diff(Option<String>),
    /// `/undo`: put back what the last turn changed.
    Undo,
    /// `/resume`: pick up the saved session of this folder.
    Resume(Option<String>),
    /// `/rename [name]`: name this session; with no name, say what it is called.
    Rename(Option<String>),
    /// `/permissions [allow <rule> | remove <rule> | dir <path>]`.
    Permissions(PermissionsCmd),
    /// `/clear`: clear the screen and start a fresh session.
    Clear,
    /// `/compact [focus]`: summarise the conversation and go on from the summary.
    Compact(Option<String>),
    /// `/context`: how full the last model's context window is.
    Context,
    /// `/cost` (also `/usage`): tokens and time of this session, by model.
    Cost,
    /// `/why`: why the router picked the last turn's model.
    Why,
    /// `/export [file]`: the conversation as Markdown.
    Export(Option<String>),
    /// `/copy`: the last answer to the clipboard.
    Copy,
    /// `/edit`: write the message in `$EDITOR`.
    Edit,
    /// `/init`: have the agent write the project's instructions file.
    Init,
    /// `/memory`: open the project's instructions file in `$EDITOR`.
    Memory,
    /// `/review [focus]`: review the uncommitted changes, read-only.
    Review(Option<String>),
    /// `/config`: the effective settings.
    Config,
    /// `/terminal-setup [install]`: Shift-Enter for a new line.
    TerminalSetup { install: bool },
    /// `!command`: run it in the shell; its output goes with the next message.
    Shell(String),
    /// `#note`: append a line to the project's AGENTS.md.
    Note(String),
    /// A slash word that is no command, with the closest real one when there is one. Never
    /// sent to the model: a typo must not cost a turn.
    Unknown { typed: String, suggestion: Option<&'static str> },
}

/// Whether provider routing starts on: `JEV_PROVIDERS=off` (also `0`, `false`, `no`)
/// starts it off, anything else, including unset, leaves the default (on).
pub fn routing_enabled(env: Option<&str>) -> bool {
    !env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

/// Levenshtein distance between two short words, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != *cb)).min(above + 1).min(row[j] + 1);
            diag = above;
        }
    }
    row[b.len()]
}

/// The command a mistyped one most likely meant: one that starts with what was typed
/// (`/mod` is `/model`), else the nearest by edit distance when it is close enough to be
/// a typo and not a different word (`/provders` is `/providers`, `/xyz` is nothing).
pub fn closest_command(typed: &str) -> Option<&'static str> {
    let typed = typed.to_ascii_lowercase();
    let names = uifmt::command_names();
    if typed.len() > 2 {
        if let Some(prefixed) = names.iter().find(|n| n.starts_with(&typed)) {
            return Some(prefixed);
        }
    }
    names
        .into_iter()
        .map(|n| (edit_distance(&typed, n), n))
        .filter(|(d, _)| *d <= 2 && *d < typed.chars().count())
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

fn usage_error(name: &str) -> Command {
    Command::Invalid(format!("usage: {}", uifmt::command_usage(name).unwrap_or_else(|| name.to_string())))
}

/// Read one input line.
pub fn parse(line: &str) -> Command {
    let line = line.trim();
    let mut words = line.split_whitespace();
    let Some(head) = words.next() else { return Command::Empty };
    let arg = words.next();
    let extra = words.next().is_some();
    let invalid = |usage: &str| Command::Invalid(usage.to_string());
    if let Some(shell) = line.strip_prefix('!') {
        let shell = shell.trim();
        return if shell.is_empty() { invalid("usage: !<command>, e.g. !git status") } else { Command::Shell(shell.to_string()) };
    }
    if let crate::gimmicks::shortcuts::Shortcut::Note(note) = crate::gimmicks::shortcuts::parse(line) {
        return Command::Note(note);
    }
    // The rest of the line after the command word, for commands that take free text.
    let rest = || {
        let r = line[head.len()..].trim();
        (!r.is_empty()).then(|| r.to_string())
    };
    let lower = head.to_ascii_lowercase();
    let head = lower.as_str();
    match head {
        "/quit" | "/exit" => Command::Quit,
        "/new" => Command::New,
        "/help" | "/status" | "/doctor" | "/undo" | "/clear" | "/context" | "/cost" | "/usage" | "/why" | "/copy" | "/edit"
        | "/init" | "/memory" | "/config"
            if arg.is_some() =>
        {
            usage_error(if head == "/usage" { "/cost" } else { head })
        }
        "/clear" => Command::Clear,
        "/compact" => Command::Compact(rest()),
        "/context" => Command::Context,
        "/cost" | "/usage" => Command::Cost,
        "/why" => Command::Why,
        "/export" => Command::Export(rest()),
        "/copy" => Command::Copy,
        "/edit" => Command::Edit,
        "/init" => Command::Init,
        "/memory" => Command::Memory,
        "/review" => Command::Review(rest()),
        "/config" => Command::Config,
        "/terminal-setup" => match (arg, extra) {
            (None, _) => Command::TerminalSetup { install: false },
            (Some("install"), false) => Command::TerminalSetup { install: true },
            _ => invalid("usage: /terminal-setup [install]"),
        },
        "/help" => Command::Help,
        "/status" => Command::Status,
        "/doctor" => Command::Doctor,
        "/undo" => Command::Undo,
        "/resume" => Command::Resume(rest()),
        "/rename" => Command::Rename(rest()),
        "/diff" => {
            // The rest of the line is one path, so a name with spaces works.
            let path = line[head.len()..].trim();
            Command::Diff((!path.is_empty()).then(|| path.to_string()))
        }
        "/permissions" => permissions::parse_args(&line[head.len()..]).map_or_else(Command::Invalid, Command::Permissions),
        "/model" => match (arg, extra) {
            (None, _) => Command::ShowModel,
            (Some(a), false) => Pin::parse(a).map_or_else(Command::Invalid, Command::SetPin),
            (Some(_), true) => invalid("usage: /model auto|<provider>|<provider>:<model>|sonnet|opus|haiku"),
        },
        "/provider" => match (arg, extra) {
            (Some("auto" | "on"), false) => Command::SetRouting(true),
            (Some("off"), false) => Command::SetRouting(false),
            _ => invalid("usage: /provider auto|off"),
        },
        "/orchestrate" => match (arg, extra) {
            (None, _) => Command::ShowOrchestrate,
            (Some("on"), false) => Command::SetOrchestrate(true),
            (Some("off"), false) => Command::SetOrchestrate(false),
            _ => invalid("usage: /orchestrate on|off"),
        },
        "/mode" => match (arg, extra) {
            (None, _) => Command::ShowMode,
            (Some(word), false) => word.parse().map_or_else(|e: crate::mode::UnknownMode| Command::Invalid(e.to_string()), Command::SetMode),
            _ => invalid("usage: /mode [ask|edit|plan|auto]"),
        },
        "/providers" => match (arg, extra) {
            (None, _) => Command::Providers { refresh: false },
            (Some("refresh"), false) => Command::Providers { refresh: true },
            _ => invalid("usage: /providers [refresh]"),
        },
        "/mcp" => match (arg, extra) {
            (None, _) | (Some("list"), false) => Command::Mcp(McpCmd::List),
            (Some("login" | "auth"), false) => invalid("usage: /mcp [list | login <server>]"),
            (Some("login" | "auth"), true) => {
                let rest_words: Vec<&str> = line[head.len()..].split_whitespace().collect();
                if rest_words.len() == 2 {
                    Command::Mcp(McpCmd::Login(rest_words[1].to_string()))
                } else {
                    invalid("usage: /mcp login <server>")
                }
            }
            _ => invalid("usage: /mcp [list | login <server>]"),
        },
        // A typo'd command must not cost a model turn; a path like `/etc/hosts` is still a prompt.
        h if h.starts_with('/') && !h[1..].contains('/') => {
            Command::Unknown { typed: head.to_string(), suggestion: closest_command(head) }
        }
        _ => Command::Prompt(line.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ProviderId;

    #[test]
    fn plain_text_is_a_prompt_and_blank_is_empty() {
        assert_eq!(parse("fix the bug"), Command::Prompt("fix the bug".into()));
        assert_eq!(parse("   "), Command::Empty);
        assert_eq!(parse("/etc/hosts looks odd"), Command::Prompt("/etc/hosts looks odd".into()));
    }

    #[test]
    fn simple_commands() {
        assert_eq!(parse("/quit"), Command::Quit);
        assert_eq!(parse("/exit"), Command::Quit);
        assert_eq!(parse("/new"), Command::New);
        assert_eq!(parse("/providers"), Command::Providers { refresh: false });
        assert_eq!(parse("/providers refresh"), Command::Providers { refresh: true });
        assert!(matches!(parse("/providers nope"), Command::Invalid(_)));
        assert_eq!(parse("/mcp"), Command::Mcp(McpCmd::List));
        assert_eq!(parse("/mcp list"), Command::Mcp(McpCmd::List));
        assert_eq!(parse("/mcp login my-server"), Command::Mcp(McpCmd::Login("my-server".into())));
        assert_eq!(parse("/mcp auth my-server"), Command::Mcp(McpCmd::Login("my-server".into())));
        assert!(matches!(parse("/mcp login"), Command::Invalid(_)));
        assert!(matches!(parse("/mcp login a b"), Command::Invalid(_)));
        assert!(matches!(parse("/mcp nope"), Command::Invalid(_)));
    }

    #[test]
    fn model_pins() {
        assert_eq!(parse("/model"), Command::ShowModel);
        assert_eq!(parse("/model auto"), Command::SetPin(Pin::Auto));
        assert_eq!(parse("/model codex"), Command::SetPin(Pin::Provider(ProviderId::Codex)));
        assert_eq!(
            parse("/model sonnet"),
            Command::SetPin(Pin::Model(ProviderId::Claude, "claude-sonnet-5-5".into()))
        );
        assert_eq!(
            parse("/model codex:gpt-6.1-sol"),
            Command::SetPin(Pin::Model(ProviderId::Codex, "gpt-6.1-sol".into()))
        );
        assert!(matches!(parse("/model nonsense"), Command::Invalid(m) if m.contains("unknown provider")));
        assert!(matches!(parse("/model codex:nope"), Command::Invalid(m) if m.contains("unknown model")));
        assert!(matches!(parse("/model a b"), Command::Invalid(_)));
    }

    #[test]
    fn routing_is_on_unless_the_env_says_off() {
        assert!(routing_enabled(None));
        assert!(routing_enabled(Some("auto")));
        assert!(routing_enabled(Some("")));
        for off in ["off", "OFF", " 0 ", "false", "no"] {
            assert!(!routing_enabled(Some(off)), "{off}");
        }
    }

    #[test]
    fn pins_print_the_way_they_are_typed() {
        for arg in ["auto", "codex", "claude:claude-opus-5-5"] {
            assert_eq!(Pin::parse(arg).unwrap().to_string(), arg);
        }
    }

    #[test]
    fn routing_toggle() {
        assert_eq!(parse("/provider auto"), Command::SetRouting(true));
        assert_eq!(parse("/provider off"), Command::SetRouting(false));
        assert!(matches!(parse("/provider"), Command::Invalid(_)));
        assert!(matches!(parse("/provider maybe"), Command::Invalid(_)));
    }

    #[test]
    fn orchestrate_toggle() {
        assert_eq!(parse("/orchestrate on"), Command::SetOrchestrate(true));
        assert_eq!(parse("/orchestrate off"), Command::SetOrchestrate(false));
        assert_eq!(parse("/orchestrate"), Command::ShowOrchestrate);
        assert_eq!(parse("/mode"), Command::ShowMode);
        assert_eq!(parse("/mode auto"), Command::SetMode(Mode::Auto));
        assert_eq!(parse("/mode acceptEdits"), Command::SetMode(Mode::Edit));
        assert!(matches!(parse("/mode yolo"), Command::Invalid(m) if m.contains("ask, edit, plan or auto")));
        assert!(matches!(parse("/mode auto plan"), Command::Invalid(_)));
        assert!(matches!(parse("/orchestrate maybe"), Command::Invalid(m) if m.contains("on|off")));
        assert!(matches!(parse("/orchestrate on now"), Command::Invalid(_)));
    }

    #[test]
    fn unknown_slash_command_is_reported_not_sent() {
        assert_eq!(parse("/provders"), Command::Unknown { typed: "/provders".into(), suggestion: Some("/providers") });
        assert_eq!(parse("/xyzzy"), Command::Unknown { typed: "/xyzzy".into(), suggestion: None });
    }

    #[test]
    fn session_and_info_commands() {
        assert_eq!(parse("/help"), Command::Help);
        assert_eq!(parse("/status"), Command::Status);
        assert_eq!(parse("/doctor"), Command::Doctor);
        assert_eq!(parse("/undo"), Command::Undo);
        assert_eq!(parse("/resume"), Command::Resume(None));
        assert_eq!(parse("/rename parser fix"), Command::Rename(Some("parser fix".into())));
        assert_eq!(parse("/rename"), Command::Rename(None));
        assert_eq!(parse("/resume fix parser"), Command::Resume(Some("fix parser".into())));
        assert_eq!(parse("/HELP"), Command::Help, "commands are not case sensitive");
        assert_eq!(parse("/help me"), Command::Invalid("usage: /help".into()));
        assert_eq!(parse("/undo now"), Command::Invalid("usage: /undo".into()));
    }

    #[test]
    fn permissions_takes_the_rest_of_the_line_as_the_rule() {
        assert_eq!(parse("/permissions"), Command::Permissions(PermissionsCmd::List));
        assert_eq!(parse("/Permissions allow Bash(cargo test:*)"), Command::Permissions(PermissionsCmd::Allow("Bash(cargo test:*)".into())));
        assert_eq!(parse("/permissions remove Bash(ls:*)"), Command::Permissions(PermissionsCmd::Remove("Bash(ls:*)".into())));
        assert_eq!(parse("/permissions dir ~/data"), Command::Permissions(PermissionsCmd::Dir("~/data".into())));
        assert!(matches!(parse("/permissions allow"), Command::Invalid(m) if m.contains("usage: /permissions")));
        assert!(matches!(parse("/permissions allow nonsense("), Command::Invalid(m) if m.contains("not a permission rule")));
    }

    #[test]
    fn new_session_and_context_commands() {
        assert_eq!(parse("/clear"), Command::Clear);
        assert_eq!(parse("/compact"), Command::Compact(None));
        assert_eq!(parse("/compact keep the API notes"), Command::Compact(Some("keep the API notes".into())));
        assert_eq!(parse("/context"), Command::Context);
        assert_eq!(parse("/cost"), Command::Cost);
        assert_eq!(parse("/usage"), Command::Cost);
        assert_eq!(parse("/usage now"), Command::Invalid("usage: /cost".into()));
        assert_eq!(parse("/why"), Command::Why);
        assert_eq!(parse("/export"), Command::Export(None));
        assert_eq!(parse("/export notes/session.md"), Command::Export(Some("notes/session.md".into())));
        assert_eq!(parse("/copy"), Command::Copy);
        assert_eq!(parse("/edit"), Command::Edit);
        assert_eq!(parse("/init"), Command::Init);
        assert_eq!(parse("/memory"), Command::Memory);
        assert_eq!(parse("/review"), Command::Review(None));
        assert_eq!(parse("/review only the parser"), Command::Review(Some("only the parser".into())));
        assert_eq!(parse("/config"), Command::Config);
        assert_eq!(parse("/terminal-setup"), Command::TerminalSetup { install: false });
        assert_eq!(parse("/terminal-setup install"), Command::TerminalSetup { install: true });
        assert!(matches!(parse("/terminal-setup now"), Command::Invalid(_)));
        assert_eq!(parse("/clear all"), Command::Invalid("usage: /clear".into()));
    }

    #[test]
    fn a_bang_runs_a_shell_command() {
        assert_eq!(parse("# remember x"), Command::Note("remember x".into()));
        assert_eq!(parse("## Heading"), Command::Prompt("## Heading".into()));
        assert_eq!(parse("!git status"), Command::Shell("git status".into()));
        assert_eq!(parse("  ! ls -la "), Command::Shell("ls -la".into()));
        assert!(matches!(parse("!"), Command::Invalid(m) if m.contains("!git status")));
    }

    #[test]
    fn diff_takes_the_rest_of_the_line_as_one_path() {
        assert_eq!(parse("/diff"), Command::Diff(None));
        assert_eq!(parse("/diff src/main.rs"), Command::Diff(Some("src/main.rs".into())));
        assert_eq!(parse("/diff  my notes/a b.md "), Command::Diff(Some("my notes/a b.md".into())));
    }

    #[test]
    fn every_command_in_the_table_is_understood() {
        // The table in uifmt drives help and completion; parsing must know each of its names.
        for name in uifmt::command_names() {
            assert!(!matches!(parse(name), Command::Unknown { .. } | Command::Prompt(_)), "{name}");
        }
    }

    #[test]
    fn edit_distance_counts_single_character_edits() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("/provders", "/providers"), 1);
        assert_eq!(edit_distance("/hlep", "/help"), 2);
    }

    #[test]
    fn the_closest_command_is_a_prefix_or_a_near_typo_and_never_a_stretch() {
        for (typed, want) in [
            ("/mod", Some("/model")),
            ("/prov", Some("/provider")),
            ("/provders", Some("/providers")),
            ("/hlep", Some("/help")),
            ("/dif", Some("/diff")),
            ("/staus", Some("/status")),
            ("/Model", Some("/model")),
            ("/xyzzy", None),
            ("/", None),
            ("/q", None),
        ] {
            assert_eq!(closest_command(typed), want, "{typed}");
        }
    }
}
