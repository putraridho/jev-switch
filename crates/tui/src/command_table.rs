//! The slash-command table: the single source that help, completion, hints and the
//! REPL's parsing all read.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Model,
    Session,
    Context,
    Project,
    Info,
}

/// One slash command: the single table that help, completion and hints all read.
pub struct Cmd {
    pub name: &'static str,
    /// The argument synopsis shown in `/help`.
    pub args: &'static str,
    pub group: Group,
    pub about: &'static str,
    /// An example line for `/help`; empty when the bare command says it all.
    pub example: &'static str,
    /// What the inline hint shows after the command.
    pub hint: &'static str,
}

pub const COMMANDS: [Cmd; 59] = [
    Cmd { name: "/model", args: "[target]", group: Group::Model, about: "Pin a model or provider, or `auto` to let the router choose", example: "/model codex:gpt-6.1-sol", hint: "auto|codex|claude:..." },
    Cmd { name: "/provider", args: "auto|off", group: Group::Model, about: "Route across providers (auto), or use Claude only (off)", example: "/provider off", hint: "auto|off" },
    Cmd { name: "/orchestrate", args: "[on|off]", group: Group::Model, about: "Let a planner split a big task across cheaper workers", example: "/orchestrate on", hint: "on|off" },
    Cmd { name: "/workflows", args: "", group: Group::Model, about: "Show orchestration: running workers, ones waiting for review, recent delegations (also /workers)", example: "", hint: "" },
    Cmd { name: "/effort", args: "[default|auto|low|medium|high]", group: Group::Model, about: "Set how hard the model thinks (a thinking budget or reasoning effort)", example: "/effort high", hint: "default|auto|low|medium|high" },
    Cmd { name: "/thinking", args: "[on|off]", group: Group::Model, about: "Print the model's whole reasoning; kept for the next session", example: "/thinking on", hint: "on|off" },
    Cmd { name: "/why", args: "", group: Group::Model, about: "Explain why the router picked the last turn's model", example: "", hint: "" },
    Cmd { name: "/new", args: "", group: Group::Session, about: "Start a fresh session on every provider", example: "", hint: "" },
    Cmd { name: "/clear", args: "", group: Group::Session, about: "Clear the screen and start a fresh session", example: "", hint: "" },
    Cmd { name: "/rename", args: "[name]", group: Group::Session, about: "Name this session, so /resume lists it by that name", example: "/rename parser fix", hint: "[name]" },
    Cmd { name: "/resume", args: "[search]", group: Group::Session, about: "Pick a previous session of this folder to continue", example: "/resume parser", hint: "[search]" },
    Cmd { name: "/sessions", args: "[search|pin|tag|export|import]", group: Group::Session, about: "Search, tag, pin, archive, export and import saved sessions of every project", example: "/sessions parser provider:codex since:7d", hint: "[search]" },
    Cmd { name: "/project", args: "[map|check|note]", group: Group::Session, about: "Project kind, repo map, test/lint with a fix loop, notes every session starts with", example: "/project check test fix", hint: "map|check|note|notes|forget" },
    Cmd { name: "/edit", args: "", group: Group::Session, about: "Write your message in $EDITOR, then send it", example: "", hint: "" },
    Cmd { name: "/diff", args: "[--all|--turn N|--review|--split|--export F] [file]", group: Group::Session, about: "Show what the agent changed (last turn, one turn, or all), review it hunk by hunk, or export a patch", example: "/diff --all", hint: "[--all|--turn N|--review] [file]" },
    Cmd { name: "/undo", args: "", group: Group::Session, about: "Undo the file changes of the last turn", example: "", hint: "" },
    Cmd { name: "/rewind", args: "[turn]", group: Group::Session, about: "Put the files back to before an earlier turn (no number: list the turns)", example: "/rewind 2", hint: "[turn]" },
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
    Cmd { name: "/help", args: "", group: Group::Info, about: "Show this help, or search it: /help <words>", example: "/help diff", hint: "" },
    Cmd { name: "/recipe", args: "[name args]", group: Group::Project, about: "List recipes, or run one: a saved sequence of steps, one turn each", example: "/recipe fix-tests the parser", hint: "[name] [args]" },
    Cmd { name: "/skill", args: "[name args | show <name> | new <name>]", group: Group::Project, about: "List skills, run one, read one, or start a new one: saved know-how you call by name", example: "/skill new db query the database", hint: "[name] [args] | show <name> | new <name>" },
    Cmd { name: "/extensions", args: "", group: Group::Project, about: "List custom commands, skills, recipes and installed extensions", example: "", hint: "" },
    Cmd { name: "/hooks", args: "[trust]", group: Group::Project, about: "Show the hooks that run on tool calls; `trust` lets this project's hooks run", example: "/hooks trust", hint: "[trust]" },
    Cmd { name: "/settings", args: "", group: Group::Info, about: "Show settings from config files and profiles, and where each came from", example: "", hint: "" },
    Cmd { name: "/commit", args: "[hint]", group: Group::Project, about: "Have the agent commit the changes, in this repo's message style", example: "/commit the parser fix", hint: "[what changed]" },
    Cmd { name: "/pr", args: "[base]", group: Group::Project, about: "Have the agent prepare (and with gh, open) a pull request", example: "/pr develop", hint: "[base branch]" },
    Cmd { name: "/team", args: "", group: Group::Project, about: "Show the repo's shared rules from .jev/team.json", example: "", hint: "" },
    Cmd { name: "/profile", args: "[name]", group: Group::Session, about: "Set a permission profile: locked, careful, standard or trusted", example: "/profile careful", hint: "locked|careful|standard|trusted" },
    Cmd { name: "/audit", args: "[n]", group: Group::Info, about: "Show the audit log: what was run, with which model, when", example: "/audit 10", hint: "[n]" },
    Cmd { name: "/history", args: "", group: Group::Info, about: "Show turns and tokens per day", example: "", hint: "" },
    Cmd { name: "/bench", args: "", group: Group::Info, about: "Rank the models by how your own turns went", example: "", hint: "" },
    Cmd { name: "/replay", args: "[n]", group: Group::Info, about: "Read a saved session back, turn by turn (n as in /resume)", example: "/replay 2", hint: "[n]" },
    Cmd { name: "/status", args: "", group: Group::Info, about: "Show the project, routing, providers and the last turn", example: "", hint: "" },
    Cmd { name: "/cost", args: "", group: Group::Info, about: "Show tokens and time used this session, by model (also /usage)", example: "", hint: "" },
    Cmd { name: "/cache", args: "", group: Group::Info, about: "Show cache hit rate per provider, when it expires, and why a turn missed", example: "", hint: "" },
    Cmd { name: "/agents", args: "", group: Group::Info, about: "Show the workers a planner handed pieces to, and their state", example: "", hint: "" },
    Cmd { name: "/orchestration", args: "", group: Group::Info, about: "Show whether orchestration is on and draw the turn as a flow", example: "", hint: "" },
    Cmd { name: "/budget", args: "[usd <n> | tokens <n> | off]", group: Group::Info, about: "Show spend by model and the plan use; set a cap on paid models", example: "/budget usd 5", hint: "[usd <n>|tokens <n>|off]" },
    Cmd { name: "/privacy", args: "[audit | local-only on|off]", group: Group::Info, about: "Show the privacy rules, the audit trail, local-only mode", example: "/privacy audit", hint: "[audit|local-only on|off]" },
    Cmd { name: "/failover", args: "[max-attempts <n> | paid-fallback on|off]", group: Group::Info, about: "Show or change how a turn fails over to another model", example: "/failover paid-fallback off", hint: "[max-attempts <n>|paid-fallback on|off]" },
    Cmd { name: "/scoreboard", args: "", group: Group::Info, about: "Show how each provider did this session: answers, failures, speed", example: "", hint: "" },
    Cmd { name: "/config", args: "", group: Group::Info, about: "Show the effective settings and where to change them", example: "/config", hint: "" },
    Cmd { name: "/providers", args: "[refresh]", group: Group::Info, about: "Show what was detected; `refresh` probes again", example: "/providers refresh", hint: "[refresh]" },
    Cmd { name: "/mcp", args: "[list|login <server>]", group: Group::Info, about: "Browse MCP servers: status, details, log in", example: "/mcp login lunagraph-remote", hint: "[list|login <server>]" },
    Cmd { name: "/tips", args: "", group: Group::Info, about: "Show short tips for getting more out of jev-code", example: "", hint: "" },
    Cmd { name: "/keys", args: "", group: Group::Info, about: "Show the key cheat sheet (also ? on an empty box)", example: "", hint: "" },
    Cmd { name: "/whatsnew", args: "[all]", group: Group::Info, about: "Show what changed in this version, or in every version", example: "/whatsnew all", hint: "[all]" },
    Cmd { name: "/doctor", args: "", group: Group::Info, about: "Check the setup and show how to fix problems", example: "", hint: "" },
    Cmd { name: "/update", args: "", group: Group::Info, about: "Update jev-code to the newest version on main and reinstall it", example: "", hint: "" },
    Cmd { name: "/terminal-setup", args: "[install]", group: Group::Info, about: "Make Shift-Enter insert a new line in your terminal", example: "/terminal-setup install", hint: "[install]" },
];

pub const KEYS: [(&str, &str); 17] = [
    ("Enter", "send; while a turn runs, queue it for after the turn"),
    ("Shift-Enter", "new line inside your message (also Alt-Enter, Ctrl-J, or \\ then Enter)"),
    ("/", "open the command menu: ↑↓ choose, Tab complete, Enter run, Esc close"),
    ("@", "pick a file or folder to point the agent at"),
    ("Esc", "stop the running turn; what you typed is sent right after (queue kept)"),
    ("Ctrl-C", "stop the running turn and drop the queue; on the box, clear it; twice on an empty box to exit"),
    ("Ctrl-D", "exit (on an empty box)"),
    ("Ctrl-V", "paste a copied image, file selection or text (Linux: wl-clipboard or xclip)"),
    ("↑ / ↓", "earlier and later messages"),
    ("Ctrl-R", "search earlier messages for what is typed"),
    ("→", "take the dim suggestion: the step the last answer offered, or the rest of an earlier message"),
    ("Ctrl-L", "clear the screen"),
    ("Shift-Tab", "cycle the mode: ask, edit, plan, auto"),
    ("!command", "run it in the shell; the output goes with your next message"),
    ("?", "on an empty box: the key cheat sheet (also /keys)"),
    ("Esc, then h j k l", "vim input mode (start with JEV_VIM=1): normal-mode keys, i/a/o to type again"),
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
        assert_eq!(command_usage("/diff").as_deref(), Some("/diff [--all|--turn N|--review|--split|--export F] [file]"));
        assert_eq!(command_usage("/new").as_deref(), Some("/new"));
        assert_eq!(command_usage("/nope"), None);
    }
}
