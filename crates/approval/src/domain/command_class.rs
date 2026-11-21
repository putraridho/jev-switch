//! Which programs and flags may never be remembered, and which only exactly.

use crate::domain::refusal::Refusal;
use crate::domain::visible_text::is_invisible;

/// Shells, interpreters, build and package runners, and wrappers that run what follows: an
/// allow rule for any of them allows every command. Matched on the program's basename, lower
/// case, with a version suffix or extension dropped (`python3.12`, `node22`, `bash.exe`).
pub(super) const RUNS_OTHER_COMMANDS: &[&str] = &[
    // shells
    "sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "ash", "rbash", "mksh", "yash", "nu", "pwsh", "powershell", "xonsh", "elvish",
    "busybox", "toybox", "rc", "es", "cmd",
    // shell builtins and keywords that run text
    "eval", "exec", "source", ".", "command", "builtin", "trap", "coproc", "if", "for", "while", "until", "case", "select", "function",
    "then", "do", "else", "elif", "fi", "done", "esac", "alias", "wait",
    // wrappers that run what follows
    "env", "sudo", "doas", "su", "pkexec", "runuser", "run0", "nohup", "nice", "ionice", "time", "timeout", "stdbuf", "setsid", "watch",
    "xargs", "parallel", "chroot", "unshare", "nsenter", "strace", "ltrace", "gdb", "lldb", "valgrind", "perf", "taskset", "chrt", "flock",
    "script", "screen", "tmux", "dtach", "at", "batch", "entr", "ssh", "mosh", "sshpass", "expect", "unbuffer", "firejail", "bwrap",
    "flatpak", "setpriv", "systemd-run", "cgexec", "faketime", "proot", "fakeroot", "caffeinate", "open", "xdg-open", "start",
    // interpreters
    "python", "python2", "python3", "pypy", "pypy3", "ipython", "node", "nodejs", "deno", "bun", "perl", "ruby", "irb", "php", "lua",
    "luajit", "tclsh", "wish", "julia", "rscript", "r", "swift", "java", "jshell", "groovy", "scala", "kotlin", "kotlinc", "dotnet",
    "awk", "gawk", "mawk", "nawk", "sed", "ts-node", "tsx", "osascript", "cscript", "wscript", "mshta", "rundll32",
    // package and build runners
    "npx", "bunx", "pnpx", "uvx", "pipx", "corepack", "make", "gmake", "bmake", "just", "rake", "task", "ninja", "ant", "gradle", "gradlew",
    "mvn", "mvnw", "bazel", "bazelisk", "buck", "scons", "cmake",
    // tools with a command or a shell escape of their own
    "find", "fd", "rg", "tar", "zip", "less", "more", "man", "vi", "vim", "nvim", "view", "nano", "emacs", "ed", "ex", "code", "git-shell",
];

/// Programs that run nothing but are refused for good, so the reason can say so: they send
/// data anywhere, delete or OVERWRITE files (`tee`, `cp`, `mv`, `ln`, `install`, `patch`,
/// `dd`), or change the system. (They would be refused by [`READ_ONLY`] anyway; this list is
/// what gives the question the reason.)
pub(super) const TOO_BROAD: &[&str] = &[
    // network and exfiltration
    "curl", "wget", "aria2c", "lftp", "nc", "ncat", "netcat", "socat", "scp", "sftp", "ftp", "tftp", "telnet", "rsync", "rclone",
    // delete, overwrite, move, link, patch
    "rm", "rmdir", "unlink", "shred", "dd", "truncate", "tee", "cp", "mv", "ln", "install", "patch", "rename", "trash", "sort", "uniq",
    "split", "csplit", "chmod", "chown", "chgrp", "setfacl", "chattr",
    // disks and the system
    "mkfs", "fdisk", "parted", "mount", "umount", "kill", "killall", "pkill", "reboot", "shutdown", "halt", "poweroff", "systemctl",
    "service", "crontab", "passwd", "useradd", "userdel", "usermod", "visudo", "iptables", "nft", "ufw", "sysctl", "modprobe", "insmod",
    "rmmod",
];

/// Programs that only read and report, which a rule may be saved for. Judged one by one for
/// the three things a rule must not allow: running another command, writing a file the
/// command line names (`sort -o`, `tree -o`, `uniq in out`, `tee`), and sending data out.
/// Left out for that reason: `sort -o`, `uniq in out`, `tree -o`, `file -C`, `date -s`,
/// `hostname name`, `less` and `bat` (a pager runs commands), `cp`, `mv`, `ln`. `jq` is kept: it
/// cannot write a file or run anything.
pub(super) const READ_ONLY: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "wc", "pwd", "echo", "printf", "stat", "du", "df", "which", "whoami", "id",
    "uname", "basename", "dirname", "realpath", "readlink", "diff", "cmp", "cut", "tr", "nl", "md5sum", "sha1sum", "sha256sum",
    "sha512sum", "b3sum", "true", "false", "sleep", "jq",
    // create, never overwrite or delete
    "mkdir", "touch",
];

/// Folders a program may be run from by its full path: the system's own, which the model
/// cannot write. A program anywhere else (`/tmp/x/ls`, `/home/u/bin/cargo`) is one it could
/// have just written.
pub(super) const SYSTEM_BIN_DIRS: &[&str] = &["/bin", "/usr/bin", "/usr/local/bin", "/sbin", "/usr/sbin", "/opt/homebrew/bin"];

/// Flags that make a read-only command write a named file or run a program, whatever the
/// program is (`git diff --output=f`, `git grep --open-files-in-pager=sh`). Matched as the
/// whole word, `flag=value`, and any abbreviation of either (`--outp=f`): option parsers
/// such as git's accept an unambiguous prefix of a long option.
pub(super) const DANGEROUS_FLAGS: &[&str] = &["--output", "--exec", "--open-files-in-pager", "--upload-pack", "--receive-pack", "--kubeconfig"];

/// Whether `word` is one of [`DANGEROUS_FLAGS`], `flag=value`, or an abbreviation of one.
pub(super) fn is_dangerous_flag(word: &str) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    name.len() > 2 && name.starts_with("--") && DANGEROUS_FLAGS.iter().any(|f| f.starts_with(name))
}

/// Subcommands that are safe as approved but take an option that WRITES A FILE the command
/// line names: every git command that parses diff options accepts `--output <file>` (checked
/// against git 2.56: `diff`, `log`, `show`, `blame`, `rev-list`, `shortlog`, `diff-tree`).
/// A rule is judged by everything it would match, and `Bash(git log:*)` would match a later
/// `git log --output=~/.bashrc`, so for these the rule is the EXACT command that was approved
/// (`Bash(git log --oneline -5)`: Claude Code reads a rule without `*` as one exact command).
pub(super) const WRITES_FILES: &[(&str, &[&str])] = &[("git", &["diff", "log", "show", "blame", "rev-list", "shortlog", "diff-tree"])];

/// Programs that are used through a subcommand, and the subcommands a rule may be saved for
/// (a rule is `Bash(git status:*)`, never `git -c ... x`). Allow-lists: a subcommand that is
/// not here is refused, which is what keeps `git push`, `git reset`, `npm install` and the
/// like out. Read-only inspection first, then the project's own build/test commands (see the
/// accepted exposures above).
pub(super) const SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("git", &[
        "status", "diff", "log", "show", "blame", "ls-files", "ls-tree", "rev-parse", "rev-list", "describe", "shortlog", "cat-file",
        "show-ref", "merge-base", "name-rev", "diff-tree", "add", "commit",
    ]),
    ("cargo", &["build", "b", "check", "c", "test", "t", "clippy", "fmt", "doc", "d", "bench", "tree", "metadata", "version"]),
    ("npm", &["test", "t", "ls", "list", "ll", "outdated", "view", "why", "explain"]),
    ("pnpm", &["test", "t", "ls", "list", "outdated", "why"]),
    ("yarn", &["test", "list", "outdated", "why", "info"]),
    ("go", &["build", "test", "vet", "list", "version"]),
    ("docker", &["ps", "images", "logs", "inspect", "version", "info"]),
    ("podman", &["ps", "images", "logs", "inspect", "version", "info"]),
    ("kubectl", &["get", "describe", "logs", "version", "top"]),
    ("pip", &["list", "show", "freeze", "check"]),
    ("brew", &["list", "info", "search", "outdated"]),
    ("apt", &["list", "show", "search", "policy"]),
    ("dnf", &["list", "info", "search"]),
];

/// Subcommands that run other commands, so the reason says so instead of "not listed".
pub(super) const RUNNING_SUBCOMMANDS: &[&str] = &[
    "run", "run-script", "exec", "x", "dlx", "create", "init", "explore", "shell", "tool", "generate", "compose", "container", "attach", "debug",
    "cp", "alias", "extension", "api", "codespace", "config", "rebase", "difftool", "mergetool", "filter-branch", "submodule", "bisect", "daemon",
    "instaweb", "hook",
];

/// The names a program can be known by for the lists above: its basename lower-cased, and
/// that without an extension and without a trailing version (`/usr/bin/python3.12` gives
/// `python3.12`, `python3` and `python`).
pub(super) fn name_forms(program: &str) -> Vec<String> {
    let base = program.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let stem = base.split('.').next().unwrap_or("").to_string();
    let unversioned = |s: &str| s.trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '_')).to_string();
    let mut forms = vec![unversioned(&base), unversioned(&stem), stem, base];
    forms.retain(|f| !f.is_empty());
    forms.dedup();
    forms
}

/// Whether the program is one a rule must not be derived from, and why.
pub(super) fn classify(program: &str) -> Result<(), Refusal> {
    let forms = name_forms(program);
    let any = |list: &[&str]| forms.iter().any(|f| list.contains(&f.as_str()));
    // `ld.so /bin/sh` runs any program.
    let loader = forms.iter().any(|f| f.starts_with("ld-") || f.starts_with("ld.so") || f.contains(".so"));
    if forms.is_empty() {
        Err(Refusal::NotPlainProgram)
    } else if any(RUNS_OTHER_COMMANDS) || loader {
        Err(Refusal::RunsOtherCommands)
    } else if any(TOO_BROAD) {
        Err(Refusal::TooBroad)
    } else {
        Ok(())
    }
}

/// A word that could be the program: letters, digits and `_ . / + -`, not a flag, and with
/// something in it (or the single `.` that `.` runs a file with).
pub(super) fn is_plain_program(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '+' | '-'))
        && (word == "." || word.chars().any(|c| c.is_ascii_alphanumeric()))
}

pub(super) fn is_subcommand(word: &str) -> bool {
    word.len() <= 24
        && word.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// A character that makes a command more than `program args`: shell syntax, a line break, a
/// control or invisible character, or whitespace that is not a plain space or tab. Quotes,
/// `#`, `~` and `* ? [ ]` are allowed: a quote only groups, a glob only expands to file names
/// (never runs anything), and a command with none of the characters below has no way to
/// start another command.
pub(super) fn is_shell_syntax(c: char) -> bool {
    matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')' | '`' | '$' | '\\' | '{' | '}' | '!')
        || (c.is_control() && c != '\t')
        || (c.is_whitespace() && c != ' ' && c != '\t')
        || is_invisible(c)
}

/// `sed -n 270,312p FILE...`: prints a range of lines and can do nothing else. `sed` as a
/// whole is refused (`-i` writes files, an `e` or `w` command runs programs or writes them),
/// so only this shape passes: `-n` first, a script that is one line number or a range of
/// them followed by `p`, and plain file names (none: it reads the pipe). A flag after the
/// script (`-i`, `-s`, `-f`) or a script with anything more in it is not this shape.
fn is_print_only_sed(words: &[&str]) -> bool {
    let [program, "-n", script, files @ ..] = words else { return false };
    let number = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let lines = script.strip_suffix('p').is_some_and(|range| match range.split_once(',') {
        Some((from, to)) => number(from) && (number(to) || to == "$"),
        None => number(range),
    });
    let plain_file = |f: &&str| !f.starts_with('-') && f.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '+' | '-' | '~' | '@'));
    *program == "sed" && lines && files.iter().all(plain_file)
}

/// `git branch -a`, `git tag -l`, `git remote -v`, `git stash list`: the listing forms of
/// commands whose other forms create, delete or rename. Only these exact flags and no names,
/// so `git branch -D x`, `git branch x`, `git tag v1`, `git remote add` and a bare
/// `git stash` (which stashes) are not listings. The rule is the exact command.
fn is_listing_git(words: &[&str]) -> bool {
    let [program, sub, flags @ ..] = words else { return false };
    let only = |allowed: &[&str]| flags.iter().all(|f| allowed.contains(f));
    *program == "git"
        && match *sub {
            "branch" => only(&["-a", "-r", "-v", "-vv", "--all", "--remotes", "--verbose", "--show-current", "--list"]),
            "tag" => only(&["-l", "--list", "-n"]),
            "remote" => only(&["-v", "--verbose"]),
            "stash" => flags == ["list"],
            _ => false,
        }
}

/// A command that only prints or lists, whose safe rule can only be the exact command (the
/// same program with other arguments would write or run something).
pub(super) fn is_listing(words: &[&str]) -> bool {
    is_print_only_sed(words) || is_listing_git(words)
}
