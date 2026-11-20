//! Why no "always" rule is offered.

// ------------------------------------------------------------------ rule derivation
//
// "Always" saves a rule that stops Claude Code asking again, and a rule is judged by what
// it would match, not by the command that was just approved. So the policy is an ALLOW-LIST
// of shapes: a plain `program [subcommand] ...` with no shell syntax anywhere in the text,
// where the program is on one of two lists and nothing else gets a rule (it is asked each
// time, y/n only, with the reason shown; `/permissions allow <rule>` is the way to write a
// rule the list does not give):
//
// - [`READ_ONLY`]: programs that read and report. None of them runs another command, takes a
//   flag that names a file to write, or sends data over a network.
// - [`SUBCOMMANDS`]: programs used through a subcommand, with the subcommands that are
//   allowed: inspection, and the build/test/check commands of the project's own toolchain.
//   Anything that pushes, publishes, installs, deletes, rewrites history, resets, or runs an
//   arbitrary script is not on the list.
//
// Everything that cannot be on either list is refused with a reason ([`RUNS_OTHER_COMMANDS`],
// [`TOO_BROAD`]) so the question says what kind of danger it is.
//
// Accepted exposures, stated here because they are decisions and not oversights:
// - `cargo build|test|check|clippy|doc|bench`, `go build|test|vet` and `npm|pnpm|yarn test`
//   run the PROJECT'S OWN code (build scripts, tests), which the model can edit. The person
//   chose this project and these are the commands they run all day; asking each time is the
//   fatigue that makes people approve without reading. `cargo fmt` rewrites project sources
//   in place, the same files the model may edit.
// - `git commit` records what is staged (an `--amend` rewrites only the local last commit,
//   recoverable from the reflog); nothing that leaves the machine is listed.
// - The read-only programs can read any file the user can, inside or outside the project
//   (`cat ~/.ssh/id_rsa`): a rule limits WHAT runs, not which paths it reads. A request for
//   a path outside the allowed folders is asked about by Claude Code itself, and then
//   "always" is judged by [`grantable_folder`].

/// Why no "always" rule is offered; [`Refusal::why`] says it in a few words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `;` `&&` `|` `>` `$(...)` backticks, a line break, control or invisible characters.
    ShellSyntax,
    /// The first word is not a plain program name (`FOO=1 make`, a quoted word, a flag).
    NotPlainProgram,
    /// A shell, an interpreter, a runner or a wrapper: a rule for it matches anything.
    RunsOtherCommands,
    /// A program that can reach too much (network, deleting files, the system).
    TooBroad,
    /// A program that takes a subcommand, called with a flag first or an executing subcommand.
    NeedsSubcommand,
    /// A program path that is not a system program (`./run.sh`, `/tmp/x/ls`): the model can
    /// rewrite it.
    ProjectProgram,
    /// A plain program or subcommand that is not on the lists of read-only and build/test
    /// commands that are safe to remember.
    NotListed,
    /// A path with wildcards, `..` or characters that would break the rule.
    UnsafePath,
    /// A subcommand that takes a file-writing option, whose rule can only be the exact
    /// command, and the command has a `*` (which a rule reads as a wildcard).
    NotExact,
    /// A hidden, settings or credentials file (`.claude/settings.local.json`, `.envrc`,
    /// `.git/hooks/x`, `secrets/x`): writing it for good could grant or run anything.
    SensitiveFile,
    /// A web address that is not a plain host name.
    BadHost,
    /// The folder that would be allowed for good is not one a grant may name: only a folder
    /// inside the home folder (never the home folder itself, never a hidden or credentials
    /// folder) or inside the system temp folder qualifies.
    FolderTooBroad,
    /// A tool for which no narrow rule exists.
    NoRule,
}

impl Refusal {
    pub fn why(self) -> &'static str {
        match self {
            Self::ShellSyntax => "it chains, pipes, redirects or expands other commands",
            Self::NotPlainProgram => "it does not start with a plain program name",
            Self::RunsOtherCommands => "it runs other commands",
            Self::TooBroad => "it could reach too much (network, deleting files, the system)",
            Self::NeedsSubcommand => "it has no plain subcommand to name",
            Self::ProjectProgram => "it runs a program that is not a system one (the model could rewrite it)",
            Self::NotListed => "it is not a read-only, build or test command that is safe to remember",
            Self::UnsafePath => "its path has wildcards, \"..\" or odd characters",
            Self::NotExact => "it can write files, so only this exact command could be remembered, and it has a \"*\"",
            Self::SensitiveFile => "it writes a hidden, settings or credentials file",
            Self::BadHost => "its address is not a plain host name",
            Self::FolderTooBroad => "the folder it touches is too broad or too sensitive to allow for good",
            Self::NoRule => "no narrow rule exists for this tool",
        }
    }

    /// The dim line that stands where the "always" choice would be.
    pub fn line(self) -> String {
        format!("can't remember this one: {}", self.why())
    }
}
