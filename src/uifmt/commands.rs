//! The slash-command table: the single source that help, completion, hints and the
//! REPL's parsing all read.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Model,
    Session,
    Info,
}

/// One slash command: the single table that help, completion and hints all read.
pub(super) struct Cmd {
    pub(super) name: &'static str,
    /// The argument synopsis shown in `/help`.
    pub(super) args: &'static str,
    pub(super) group: Group,
    pub(super) about: &'static str,
    /// An example line for `/help`; empty when the bare command says it all.
    pub(super) example: &'static str,
    /// What the inline hint shows after the command.
    pub(super) hint: &'static str,
}

pub(super) const COMMANDS: [Cmd; 13] = [
    Cmd { name: "/model", args: "[target]", group: Group::Model, about: "Pin a model or provider, or `auto` to let the router choose", example: "/model codex:gpt-6.1-sol", hint: "auto|codex|claude:..." },
    Cmd { name: "/provider", args: "auto|off", group: Group::Model, about: "Route across providers (auto), or use Claude only (off)", example: "/provider off", hint: "auto|off" },
    Cmd { name: "/orchestrate", args: "[on|off]", group: Group::Model, about: "Let a planner split a big task across cheaper workers", example: "/orchestrate on", hint: "on|off" },
    Cmd { name: "/new", args: "", group: Group::Session, about: "Start a fresh session on every provider", example: "", hint: "" },
    Cmd { name: "/resume", args: "", group: Group::Session, about: "Pick up a previous session", example: "", hint: "" },
    Cmd { name: "/diff", args: "[file]", group: Group::Session, about: "Show what the agent changed, all files or one", example: "/diff src/main.rs", hint: "[file]" },
    Cmd { name: "/undo", args: "", group: Group::Session, about: "Undo the file changes of the last turn", example: "", hint: "" },
    Cmd { name: "/permissions", args: "[allow|remove|dir <x>]", group: Group::Session, about: "Show or change which tool calls are allowed without asking", example: "/permissions allow Bash(cargo test:*)", hint: "allow|remove|dir" },
    Cmd { name: "/quit", args: "", group: Group::Session, about: "Exit (also /exit, or Ctrl-D)", example: "", hint: "" },
    Cmd { name: "/help", args: "", group: Group::Info, about: "Show this help", example: "", hint: "" },
    Cmd { name: "/status", args: "", group: Group::Info, about: "Show the project, routing, providers and the last turn", example: "", hint: "" },
    Cmd { name: "/providers", args: "[refresh]", group: Group::Info, about: "Show what was detected; `refresh` probes again", example: "/providers refresh", hint: "[refresh]" },
    Cmd { name: "/doctor", args: "", group: Group::Info, about: "Check the setup and show how to fix problems", example: "", hint: "" },
];

pub(super) const KEYS: [(&str, &str); 5] = [
    ("Ctrl-C", "stop the running turn"),
    ("Ctrl-D", "exit (on an empty line)"),
    ("Alt-Enter", "newline inside your message"),
    ("↑ / ↓", "earlier and later messages"),
    ("Tab", "complete a command or its argument"),
];

pub(super) fn usage(c: &Cmd) -> String {
    if c.args.is_empty() {
        c.name.to_string()
    } else {
        format!("{} {}", c.name, c.args)
    }
}

/// Every slash command name (`/model`, ...), from the one table that help, completion and
/// parsing share.
pub fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|c| c.name).collect()
}

/// `/diff [file]`: the command with its argument synopsis, or `None` for a name that is
/// not in the table.
pub fn command_usage(name: &str) -> Option<String> {
    COMMANDS.iter().find(|c| c.name == name).map(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_table_gives_names_and_usage_to_parsing() {
        let names = command_names();
        assert!(names.contains(&"/model") && names.contains(&"/help") && names.contains(&"/undo"));
        assert_eq!(command_usage("/diff").as_deref(), Some("/diff [file]"));
        assert_eq!(command_usage("/new").as_deref(), Some("/new"));
        assert_eq!(command_usage("/nope"), None);
    }
}
