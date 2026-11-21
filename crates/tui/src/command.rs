//! What one line typed into `jev-code` means. Pure, so the REPL binary only maps
//! each [`Command`] onto the [`jev_turn::application::engine::Engine`] and every spelling rule is a
//! unit test. The names come from the one command table in [`crate::render`], which also
//! drives `/help` and Tab completion.

pub mod typo;

use typo::closest_command;
use crate::permissions_command;
use crate::repl::guard_commands::GuardCmd;
use jev_approval::application::extensions as ext;
use jev_approval::application::extensions::ExtCmd;
use jev_approval::domain::mode::Mode;
use crate::permissions_command::PermissionsCmd;
use jev_routing::domain::decide::Pin;
use crate::render as uifmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpCmd {
    /// `/mcp` or `/mcp login`: the interactive list (the table when there is no terminal).
    Browse,
    /// `/mcp list`: the plain table.
    List,
    /// `/mcp login <server>`.
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
    /// `/thinking on|off`: print the model's whole reasoning, for this and later sessions.
    SetThinking(bool),
    /// `/thinking` with no argument: show whether it is on.
    ShowThinking,
    /// `/mode` with no argument: show the mode.
    ShowMode,
    /// `/mode <name>`.
    SetMode(Mode),
    /// `/providers [refresh]`.
    Providers { refresh: bool },
    /// `/mcp [list | login <server>]`.
    Mcp(McpCmd),
    /// `/commit`, `/pr`, `/profile`, `/team`, `/audit`, `/history`, `/bench`, `/replay`.
    Trust(crate::repl::extended_commands::ExtCmd),
    /// A command that was recognised but whose argument was not; the text says what is accepted.
    Invalid(String),
    /// `/help`.
    Help,
    /// `/help <words>`, `/tips`, `/keys`, `/whatsnew`.
    Learn(crate::views::learn::Learn),
    /// `/status`.
    Status,
    /// `/doctor`: the setup check, inside the session.
    Doctor,
    /// `/update`: pull the newest version of jev-code's checkout and reinstall it.
    Update,
    /// `/diff [file]`: what the last turn changed.
    Diff(Option<String>),
    /// `/undo`: put back what the last turn changed.
    Undo,
    /// `/rewind [turn]`: put the files back to before that turn (none: list the turns).
    Rewind(Option<usize>),
    /// `/resume`: pick up the saved session of this folder.
    Resume(Option<String>),
    /// `/sessions ...`: the library of saved sessions.
    Sessions(crate::repl::library_command::SessionsCmd),
    /// `/project ...`: kind, repo map, checks and notes of this project.
    Project(crate::repl::project_command::ProjectCmd),
    /// `/rename [name]`: name this session; with no name, say what it is called.
    Rename(Option<String>),
    /// `/workflows` (also `/workers`): what orchestration is doing in this project.
    Workflows,
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
    /// `/budget`, `/privacy`, `/failover`, `/scoreboard`: what fences in and watches the router.
    Guard(GuardCmd),
    /// `/why`: why the router picked the last turn's model.
    Why,
    /// `/cache`: cache hit rate per provider, window countdown, named misses.
    Cache,
    /// `/effort [setting]`: show or set the reasoning effort.
    Effort(Option<jev_turn::domain::effort::EffortSetting>),
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
    /// `/agents`: the workers a planner handed pieces to.
    Agents,
    /// `/orchestration`: whether orchestration is on, and the turn as a flow diagram.
    Orchestration,
    /// `/terminal-setup [install]`: Shift-Enter for a new line.
    TerminalSetup { install: bool },
    /// `!command`: run it in the shell; its output goes with the next message.
    Shell(String),
    /// `#note`: append a line to the project's AGENTS.md.
    Note(String),
    /// A slash word that is no command, with the closest real one when there is one. Never
    /// sent to the model: a typo must not cost a turn.
    /// `args` is the rest of the line: a custom command or recipe of that name may take it.
    Unknown { typed: String, args: String, suggestion: Option<&'static str> },
    /// `/extensions`, `/recipe`, `/skill`, `/hooks`.
    Ext(ExtCmd),
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
    if let crate::input::shortcuts::Shortcut::Note(note) = crate::input::shortcuts::parse(line) {
        return Command::Note(note);
    }
    // The rest of the line after the command word, for commands that take free text.
    let rest = || {
        let r = line[head.len()..].trim();
        (!r.is_empty()).then(|| r.to_string())
    };
    let lower = head.to_ascii_lowercase();
    let head = lower.as_str();
    if let Some(learn) = crate::views::learn::Learn::parse(head, rest().as_deref()) {
        return learn.map_or_else(Command::Invalid, Command::Learn);
    }
    if let Some(ext) = crate::repl::extended_commands::parse(head, &line[head.len()..]) {
        return Command::Trust(ext);
    }
    match head {
        "/quit" | "/exit" => Command::Quit,
        "/new" => Command::New,
        "/help" | "/status" | "/doctor" | "/update" | "/undo" | "/clear" | "/context" | "/cost" | "/usage" | "/why" | "/copy" | "/edit" | "/cache"
        | "/init" | "/memory" | "/config" | "/agents" | "/orchestration"
            if arg.is_some() =>
        {
            usage_error(if head == "/usage" { "/cost" } else { head })
        }
        "/clear" => Command::Clear,
        "/compact" => Command::Compact(rest()),
        "/context" => Command::Context,
        "/cost" | "/usage" => Command::Cost,
        "/budget" | "/privacy" | "/failover" | "/scoreboard" => {
            GuardCmd::parse(head, &line[head.len()..]).map_or_else(Command::Invalid, Command::Guard)
        }
        "/why" => Command::Why,
        "/cache" => Command::Cache,
        "/effort" => match (arg, extra) {
            (None, _) => Command::Effort(None),
            (Some(word), false) => word.parse().map_or_else(Command::Invalid, |s| Command::Effort(Some(s))),
            _ => invalid("usage: /effort [default|auto|low|medium|high]"),
        },
        "/export" => Command::Export(rest()),
        "/copy" => Command::Copy,
        "/edit" => Command::Edit,
        "/init" => Command::Init,
        "/memory" => Command::Memory,
        "/review" => Command::Review(rest()),
        "/config" => Command::Config,
        "/agents" => Command::Agents,
        "/orchestration" => Command::Orchestration,
        "/terminal-setup" => match (arg, extra) {
            (None, _) => Command::TerminalSetup { install: false },
            (Some("install"), false) => Command::TerminalSetup { install: true },
            _ => invalid("usage: /terminal-setup [install]"),
        },
        "/help" => Command::Help,
        "/status" => Command::Status,
        "/doctor" => Command::Doctor,
        "/update" => Command::Update,
        "/undo" => Command::Undo,
        "/rewind" => match (arg, extra) {
            (None, _) => Command::Rewind(None),
            (Some(n), false) => n.parse::<usize>().ok().filter(|n| *n > 0).map_or_else(|| invalid("usage: /rewind [turn number]"), |n| Command::Rewind(Some(n))),
            (Some(_), true) => invalid("usage: /rewind [turn number]"),
        },
        "/resume" => Command::Resume(rest()),
        "/rename" => Command::Rename(rest()),
        "/workflows" | "/workers" => Command::Workflows,
        "/sessions" => crate::repl::library_command::parse(&line[head.len()..]).map_or_else(|e| Command::Invalid(e.to_string()), Command::Sessions),
        "/project" => crate::repl::project_command::parse(&line[head.len()..]).map_or_else(Command::Invalid, Command::Project),
        "/diff" => {
            // The rest of the line is one path, so a name with spaces works.
            let path = line[head.len()..].trim();
            Command::Diff((!path.is_empty()).then(|| path.to_string()))
        }
        "/permissions" => permissions_command::parse_args(&line[head.len()..]).map_or_else(Command::Invalid, Command::Permissions),
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
        "/thinking" => match (arg, extra) {
            (None, _) => Command::ShowThinking,
            (Some("on"), false) => Command::SetThinking(true),
            (Some("off"), false) => Command::SetThinking(false),
            _ => invalid("usage: /thinking on|off"),
        },
        "/mode" => match (arg, extra) {
            (None, _) => Command::ShowMode,
            (Some(word), false) => word.parse().map_or_else(|e: jev_approval::domain::mode::UnknownMode| Command::Invalid(e.to_string()), Command::SetMode),
            _ => invalid("usage: /mode [ask|edit|plan|auto]"),
        },
        "/providers" => match (arg, extra) {
            (None, _) => Command::Providers { refresh: false },
            (Some("refresh"), false) => Command::Providers { refresh: true },
            _ => invalid("usage: /providers [refresh]"),
        },
        "/mcp" => match (arg, extra) {
            (None, _) | (Some("login" | "auth"), false) => Command::Mcp(McpCmd::Browse),
            (Some("list"), false) => Command::Mcp(McpCmd::List),
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
        "/extensions" | "/recipe" | "/skill" | "/hooks" | "/settings" => {
            ext::parse_command(head, rest().as_deref()).map_or(Command::Empty, |r| r.map_or_else(Command::Invalid, Command::Ext))
        }
        h if h.starts_with('/') && !h[1..].contains('/') => {
            Command::Unknown { typed: head.to_string(), args: rest().unwrap_or_default(), suggestion: closest_command(head) }
        }
        _ => Command::Prompt(line.to_string()),
    }
}


#[cfg(test)]
mod tests;
