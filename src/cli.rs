//! The command line of `jev-code`: `jev-code [options] [dir]`, `jev-code doctor [dir]`.
//! Pure: [`parse`] maps the arguments to an [`Args`] or a one-line usage error, so every
//! spelling is a unit test and the binary never panics on a bad flag.

use std::path::PathBuf;

use crate::mode::Mode;
use crate::routing::Pin;

/// Where a one-shot prompt (`-p`) comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSource {
    /// `-p "fix the bug"`.
    Text(String),
    /// `-p -`: read all of standard input.
    Stdin,
}

/// What the program was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The interactive session (or a piped one, when stdin is not a terminal).
    Session,
    /// `-p`: one turn; only the answer on stdout.
    Print(PromptSource),
    /// `doctor`: check the setup.
    Doctor,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub action: Action,
    /// The project directory; the current one when absent.
    pub dir: Option<PathBuf>,
    /// `-c`: resume the saved session of the directory.
    pub resume: bool,
    /// `--no-color`.
    pub no_color: bool,
    /// `--model <target>`: the pin to start with (as `/model`).
    pub pin: Option<Pin>,
    /// `--mode <mode>`: the permission mode to start in (as `/mode`).
    pub mode: Option<Mode>,
}

/// A command line that cannot be run; the text is one line, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// The short usage printed under a usage error.
pub const USAGE: &str = "usage: jev-code [options] [dir]\n       jev-code doctor [dir]\ntry `jev-code --help`";

fn err<T>(message: impl Into<String>) -> Result<T, UsageError> {
    Err(UsageError(message.into()))
}

/// Read the arguments after the program name.
pub fn parse<I, S>(args: I) -> Result<Args, UsageError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut it = args.into_iter().map(Into::into);
    let mut out = Args { action: Action::Session, dir: None, resume: false, no_color: false, pin: None, mode: None };
    let mut doctor = false;
    let mut print: Option<PromptSource> = None;
    let mut only_positionals = false;
    while let Some(arg) = it.next() {
        if only_positionals || !arg.starts_with('-') || arg == "-" {
            if arg == "doctor" && !doctor && out.dir.is_none() && print.is_none() {
                doctor = true;
            } else if out.dir.is_some() {
                return err(format!("unexpected argument `{arg}`: only one project directory can be given"));
            } else {
                out.dir = Some(PathBuf::from(arg));
            }
            continue;
        }
        match arg.as_str() {
            "--" => only_positionals = true,
            "-h" | "--help" => {
                out.action = Action::Help;
                return Ok(out);
            }
            "-v" | "-V" | "--version" => {
                out.action = Action::Version;
                return Ok(out);
            }
            "-c" | "--continue" => out.resume = true,
            "--no-color" => out.no_color = true,
            "-p" | "--print" => {
                let Some(value) = it.next() else { return err(format!("{arg} needs a prompt (or `-` to read it from stdin)")) };
                print = Some(prompt_source(value)?);
            }
            long if long.starts_with("--print=") => print = Some(prompt_source(long["--print=".len()..].to_string())?),
            "-m" | "--model" => {
                let Some(value) = it.next() else { return err(format!("{arg} needs a model, e.g. {arg} codex or {arg} opus")) };
                out.pin = Some(Pin::parse(&value).map_err(UsageError)?);
            }
            long if long.starts_with("--model=") => out.pin = Some(Pin::parse(&long["--model=".len()..]).map_err(UsageError)?),
            "--mode" => {
                let Some(value) = it.next() else { return err("--mode needs a mode: ask, edit, plan or auto") };
                out.mode = Some(value.parse().map_err(|e: crate::mode::UnknownMode| UsageError(e.to_string()))?);
            }
            long if long.starts_with("--mode=") => {
                out.mode = Some(long["--mode=".len()..].parse().map_err(|e: crate::mode::UnknownMode| UsageError(e.to_string()))?);
            }
            other => return err(format!("unknown option {other}")),
        }
    }
    out.action = match (doctor, print) {
        (true, Some(_)) => return err("`doctor` and -p cannot be combined"),
        (true, None) => Action::Doctor,
        (false, Some(source)) => Action::Print(source),
        (false, None) => Action::Session,
    };
    Ok(out)
}

fn prompt_source(value: String) -> Result<PromptSource, UsageError> {
    match value.as_str() {
        "-" => Ok(PromptSource::Stdin),
        v if v.trim().is_empty() => err("the -p prompt is empty"),
        _ => Ok(PromptSource::Text(value)),
    }
}

/// `--help`.
pub const HELP: &str = "jev-code: a coding agent that routes each turn across the AI coding tools you have

USAGE:
  jev-code [options] [project-dir]   start in project-dir (default: the current directory)
  jev-code doctor [project-dir]      check the setup and say how to fix what is missing

OPTIONS:
  -p, --print <prompt>   run one turn, print only the answer on stdout (progress goes to
                         stderr); `-p -` reads the prompt from stdin. Exit code: 0 answered,
                         1 failed, 130 stopped with Ctrl-C
  -c, --continue         resume the saved session of this directory
  -m, --model <target>   start pinned to a model or provider, as /model (codex, opus,
                         claude:claude-sonnet-5-5, auto)
      --mode <mode>      start in a permission mode, as /mode: ask, edit, plan or auto
      --no-color         no colours or styling (also: NO_COLOR=1)
  -h, --help             this text
  -v, -V, --version      version

IN SESSION: type /help for the commands and keys. Tab completes commands, models and
@paths; Esc or Ctrl-C stops the running turn (Ctrl-C twice at the prompt, or Ctrl-D, to
exit); Alt-Enter or \\ then Enter adds a line; Enter while a turn runs queues the message;
!command runs a shell command; drop a file or image into the terminal to attach it.

A provider is used only if it is installed and logged in (or has its key), is not
cooling down after a usage limit, and its billing is allowed. When a turn hits a limit
or fails, the next best model gets the same prompt; the failed attempt's output is
never shown as an answer. Security work never goes to DeepSeek or Grok.

ENVIRONMENT (each may also be a KEY=VALUE line in ~/.jev-router.env):
  TYPESAFE_API_KEY | JEV_API_KEY   the Jev key (required for routing)
  JEV_TYPESAFE_URL           a different Jev endpoint (proxy, test server)
  JEV_ORCHESTRATE            on starts with orchestration on; default off
  JEV_DELEGATE_BIN           the delegate tool (default: jev-delegate-mcp or delegate_mcp next to jev-code)
  JEV_DELEGATE_LOG           orchestration log (default <state dir>/orchestration.jsonl)
  JEV_PROVIDERS              off starts with routing off (Claude only); default on
  JEV_ALLOW_CREDITS          1 allows models billed as extra credits (e.g. Claude Fable); default off
  JEV_CHAINS                 JSON file replacing the model order per task type
  DEEPSEEK_API_KEY           enables DeepSeek (billed per token; used through the claude CLI)
  JEV_TIMEOUT_MS             longest wait for Jev before routing without it (default 3000)
  CODE_IDLE_TIMEOUT_SECS     stop a turn when the CLI prints nothing for this long (default 900)
  JEV_CODE_LOG               turn log file (default <state dir>/code-turns.jsonl)
  CODE_PERMISSION_MODE       Claude Code permission mode (default acceptEdits)
  CODE_ALLOWED_TOOLS         extra tools Claude Code may use, e.g. \"Bash(cargo test) Bash(git status)\"
  CLAUDE_BIN, CODEX_BIN, GROK_BIN, CODEX_SANDBOX   binary paths and the Codex sandbox
  CODE_STRONG_THRESHOLD      with routing off: Jev reasoning confidence for Opus (default 0.85)
  DOWNGRADE_AFTER            with routing off: easier turns before stepping back to Sonnet (default 2)
  PAGER                      what /diff pipes long output through (default: less -R)
  VISUAL | EDITOR            the editor of /edit and /memory (default: vi)
  JEV_BELL                   off: no bell when a long turn ends
  NO_COLOR                   set to turn colours off

In the default edit mode, file edits are applied without asking (commands still ask), so
use it inside a git repo; /undo puts back what the last turn changed.";

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(args: &[&str]) -> Args {
        parse(args.iter().copied()).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn bad(args: &[&str]) -> String {
        parse(args.iter().copied()).expect_err(&format!("{args:?} should not parse")).0
    }

    #[test]
    fn no_arguments_is_a_session_in_the_current_directory() {
        assert_eq!(ok(&[]), Args { action: Action::Session, dir: None, resume: false, no_color: false, pin: None, mode: None });
    }

    #[test]
    fn a_directory_is_the_one_positional_argument() {
        assert_eq!(ok(&["/tmp/proj"]).dir, Some(PathBuf::from("/tmp/proj")));
        assert!(bad(&["a", "b"]).contains("unexpected argument `b`"));
    }

    #[test]
    fn flags_and_the_directory_can_come_in_any_order() {
        let a = ok(&["-c", "proj", "--no-color"]);
        assert!(a.resume && a.no_color);
        assert_eq!(a.dir, Some(PathBuf::from("proj")));
        let b = ok(&["proj", "--continue"]);
        assert!(b.resume);
        assert_eq!(b.dir, Some(PathBuf::from("proj")));
    }

    #[test]
    fn print_takes_a_prompt_or_stdin() {
        assert_eq!(ok(&["-p", "say hi"]).action, Action::Print(PromptSource::Text("say hi".into())));
        assert_eq!(ok(&["--print", "say hi"]).action, Action::Print(PromptSource::Text("say hi".into())));
        assert_eq!(ok(&["--print=say hi"]).action, Action::Print(PromptSource::Text("say hi".into())));
        assert_eq!(ok(&["-p", "-"]).action, Action::Print(PromptSource::Stdin));
        // The value is taken as is, even when it looks like a flag.
        assert_eq!(ok(&["-p", "--help"]).action, Action::Print(PromptSource::Text("--help".into())));
        assert_eq!(ok(&["-p", "x", "dir", "-c"]).dir, Some(PathBuf::from("dir")));
    }

    #[test]
    fn print_without_a_usable_prompt_is_a_usage_error() {
        assert!(bad(&["-p"]).contains("needs a prompt"));
        assert!(bad(&["-p", "  "]).contains("empty"));
        assert!(bad(&["--print="]).contains("empty"));
    }

    #[test]
    fn doctor_is_a_subcommand_with_an_optional_directory() {
        assert_eq!(ok(&["doctor"]).action, Action::Doctor);
        let a = ok(&["doctor", "proj"]);
        assert_eq!((a.action, a.dir), (Action::Doctor, Some(PathBuf::from("proj"))));
        assert_eq!(ok(&["--no-color", "doctor"]).action, Action::Doctor);
        assert!(bad(&["doctor", "-p", "x"]).contains("cannot be combined"));
        // After -p the word is just a directory.
        assert_eq!(ok(&["-p", "x", "doctor"]).dir, Some(PathBuf::from("doctor")));
        // A directory that happens to be called doctor is reachable.
        assert_eq!(ok(&["./doctor"]).dir, Some(PathBuf::from("./doctor")));
        assert_eq!(ok(&["--", "doctor"]).action, Action::Doctor);
    }

    #[test]
    fn help_and_version_win_at_once() {
        assert_eq!(ok(&["-h"]).action, Action::Help);
        assert_eq!(ok(&["--help"]).action, Action::Help);
        assert_eq!(ok(&["-V"]).action, Action::Version);
        assert_eq!(ok(&["-v"]).action, Action::Version);
        assert_eq!(ok(&["--version"]).action, Action::Version);
        assert_eq!(ok(&["proj", "--help"]).action, Action::Help);
    }

    #[test]
    fn unknown_flags_are_usage_errors_not_panics() {
        assert_eq!(bad(&["--frobnicate"]), "unknown option --frobnicate");
        assert_eq!(bad(&["-x"]), "unknown option -x");
        assert!(bad(&["dir", "--nope"]).contains("--nope"));
    }

    #[test]
    fn model_and_mode_set_how_the_session_starts() {
        use crate::backend::ProviderId;
        assert_eq!(ok(&["--model", "codex"]).pin, Some(Pin::Provider(ProviderId::Codex)));
        assert_eq!(ok(&["-m", "opus", "proj"]).pin, Some(Pin::Model(ProviderId::Claude, "claude-opus-5-5".into())));
        assert_eq!(ok(&["--model=auto"]).pin, Some(Pin::Auto));
        assert_eq!(ok(&["--mode", "plan"]).mode, Some(Mode::Plan));
        assert_eq!(ok(&["--mode=auto"]).mode, Some(Mode::Auto));
        assert!(bad(&["--model"]).contains("needs a model"));
        assert!(bad(&["--model", "nonsense"]).contains("unknown provider"));
        assert!(bad(&["--mode", "yolo"]).contains("ask, edit, plan or auto"));
    }

    #[test]
    fn double_dash_makes_the_rest_positional() {
        assert_eq!(ok(&["--", "-weird-dir"]).dir, Some(PathBuf::from("-weird-dir")));
    }

    #[test]
    fn a_lone_dash_is_a_positional_not_a_flag() {
        assert_eq!(ok(&["-"]).dir, Some(PathBuf::from("-")));
    }

    #[test]
    fn help_mentions_every_option_the_parser_accepts() {
        for flag in ["-p, --print", "-c, --continue", "-m, --model", "--mode", "--no-color", "-h, --help", "-v, -V, --version", "doctor"] {
            assert!(HELP.contains(flag), "{flag}");
        }
    }
}
