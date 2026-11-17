//! The slash-command table: the single source that help, completion, hints and the
//! REPL's parsing all read.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    Model,
    Session,
    Context,
    Project,
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

pub(super) const COMMANDS: [Cmd; 27] = [
    Cmd { name: "/model", args: "[target]", group: Group::Model, about: "Pin a model or provider, or `auto` to let the router choose", example: "/model codex:gpt-6.1-sol", hint: "auto|codex|claude:..." },
    Cmd { name: "/provider", args: "auto|off", group: Group::Model, about: "Route across providers (auto), or use Claude only (off)", example: "/provider off", hint: "auto|off" },
    Cmd { name: "/orchestrate", args: "[on|off]", group: Group::Model, about: "Let a planner split a big task across cheaper workers", example: "/orchestrate on", hint: "on|off" },
    Cmd { name: "/why", args: "", group: Group::Model, about: "Explain why the router picked the last turn's model", example: "", hint: "" },
    Cmd { name: "/new", args: "", group: Group::Session, about: "Start a fresh session on every provider", example: "", hint: "" },
    Cmd { name: "/clear", args: "", group: Group::Session, about: "Clear the screen and start a fresh session", example: "", hint: "" },
    Cmd { name: "/resume", args: "", group: Group::Session, about: "Pick up a previous session", example: "", hint: "" },
    Cmd { name: "/edit", args: "", group: Group::Session, about: "Write your message in $EDITOR, then send it", example: "", hint: "" },
    Cmd { name: "/diff", args: "[file]", group: Group::Session, about: "Show what the agent changed, all files or one", example: "/diff src/main.rs", hint: "[file]" },
    Cmd { name: "/undo", args: "", group: Group::Session, about: "Undo the file changes of the last turn", example: "", hint: "" },
    Cmd { name: "/mode", args: "[ask|edit|plan|auto]", group: Group::Session, about: "Set how much the agent may do without asking (Shift-Tab cycles it)", example: "/mode auto", hint: "ask|edit|plan|auto" },
    Cmd { name: "/permissions", args: "[allow|remove|dir <x>]", group: Group::Session, about: "Show or change which tool calls are allowed without asking", example: "/permissions allow Bash(cargo test:*)", hint: "allow|remove|dir" },
    Cmd { name: "/quit", args: "", group: Group::Session, about: "Exit (also /exit, or Ctrl-D)", example: "", hint: "" },
    Cmd { name: "/compact", args: "[focus]", group: Group::Context, about: "Summarise the conversation and go on from the summary (frees context)", example: "/compact keep the API decisions", hint: "[what to keep]" },
    Cmd { name: "/context", args: "", group: Group::Context, about: "Show how full the model's context window is", example: "", hint: "" },
    Cmd { name: "/copy", args: "", group: Group::Context, about: "Copy the last answer to the clipboard", example: "", hint: "" },
    Cmd { name: "/export", args: "[file]", group: Group::Context, about: "Save this session's conversation as Markdown", example: "/export notes/session.md", hint: "[file]" },
    Cmd { name: "/init", args: "", group: Group::Project, about: "Have the agent write AGENTS.md, the instructions every CLI reads", example: "", hint: "" },
    Cmd { name: "/memory", args: "", group: Group::Project, about: "Open the project's instructions (AGENTS.md, CLAUDE.md) in $EDITOR", example: "", hint: "" },
    Cmd { name: "/review", args: "[focus]", group: Group::Project, about: "Review the uncommitted changes for bugs, without editing", example: "/review the parser changes", hint: "[focus]" },
    Cmd { name: "/help", args: "", group: Group::Info, about: "Show this help", example: "", hint: "" },
    Cmd { name: "/status", args: "", group: Group::Info, about: "Show the project, routing, providers and the last turn", example: "", hint: "" },
    Cmd { name: "/cost", args: "", group: Group::Info, about: "Show tokens and time used this session, by model (also /usage)", example: "", hint: "" },
    Cmd { name: "/config", args: "", group: Group::Info, about: "Show the effective settings and where to change them", example: "", hint: "" },
    Cmd { name: "/providers", args: "[refresh]", group: Group::Info, about: "Show what was detected; `refresh` probes again", example: "/providers refresh", hint: "[refresh]" },
    Cmd { name: "/doctor", args: "", group: Group::Info, about: "Check the setup and show how to fix problems", example: "", hint: "" },
    Cmd { name: "/terminal-setup", args: "[install]", group: Group::Info, about: "Make Shift-Enter insert a new line in your terminal", example: "/terminal-setup install", hint: "[install]" },
];

pub(super) const KEYS: [(&str, &str); 13] = [
    ("Enter", "send; while a turn runs, queue the message for after it"),
    ("Alt-Enter, \\ Enter", "new line inside your message (Shift-Enter after /terminal-setup)"),
    ("Esc", "stop the running turn; what you typed is sent right after (queue kept)"),
    ("Ctrl-C", "stop the running turn and drop the queue; on the prompt, twice to exit"),
    ("Ctrl-D", "exit (on an empty line)"),
    ("↑ / ↓", "earlier and later messages"),
    ("Ctrl-R", "search earlier messages"),
    ("Ctrl-L", "clear the screen"),
    ("Tab", "complete a command, its argument, or an @path"),
    ("Shift-Tab", "cycle the mode: ask, edit, plan, auto"),
    ("!command", "run it in the shell; the output goes with your next message"),
    ("@path", "point the agent at a file or folder (Tab completes)"),
    ("drop a file", "drag a file or image into the terminal to attach it"),
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
