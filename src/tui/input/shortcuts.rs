//! Input shortcuts: `!cmd` runs a shell command and shows its output without involving a
//! model; `#note` appends a note to the project's memory file.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a line of input means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortcut {
    Shell(String),
    Note(String),
    /// Not a shortcut: send it as a message or slash command.
    Plain,
}

/// `!ls` is a shell command and `# remember x` a note; a bare `!` or `#` (nothing after
/// it), `!!` and `#!` lines (shebang-like or Markdown headings the user pasted) are plain.
pub fn parse(line: &str) -> Shortcut {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix('!') {
        let cmd = rest.trim();
        return if cmd.is_empty() || rest.starts_with('!') { Shortcut::Plain } else { Shortcut::Shell(cmd.to_string()) };
    }
    if let Some(rest) = line.strip_prefix('#') {
        let note = rest.trim();
        return if note.is_empty() || rest.starts_with(['#', '!']) { Shortcut::Plain } else { Shortcut::Note(note.to_string()) };
    }
    Shortcut::Plain
}

/// What a shell command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellResult {
    pub output: String,
    pub success: bool,
}

/// The most output kept from a shell command (the rest is counted).
pub const MAX_SHELL_OUTPUT: usize = 20_000;

/// Run `cmd` through `sh -c` in `cwd`, stdout and stderr together, with no input.
pub fn run_shell(cmd: &str, cwd: &Path) -> std::io::Result<ShellResult> {
    let out = Command::new("sh").arg("-c").arg(format!("exec 2>&1\n{cmd}")).current_dir(cwd).stdin(Stdio::null()).output()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.len() > MAX_SHELL_OUTPUT {
        let mut cut = MAX_SHELL_OUTPUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        let dropped = text.len() - cut;
        text.truncate(cut);
        text.push_str(&format!("\n… {dropped} more bytes"));
    }
    Ok(ShellResult { output: text, success: out.status.success() })
}

/// Append `- note` to the memory file at `path`, creating it (and its folder) when needed.
pub fn append_note(path: &Path, note: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let one_line: String = note.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let needs_break = std::fs::read(path).is_ok_and(|b| !b.is_empty() && !b.ends_with(b"\n"));
    if needs_break {
        f.write_all(b"\n")?;
    }
    writeln!(f, "- {one_line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        crate::testkit::scratch_dir::scratch_dir("shortcuts", name)
    }

    #[test]
    fn parsing_table() {
        for (line, want) in [
            ("!ls -la", Shortcut::Shell("ls -la".into())),
            ("  ! git status ", Shortcut::Shell("git status".into())),
            ("# remember: use tabs", Shortcut::Note("remember: use tabs".into())),
            ("#todo", Shortcut::Note("todo".into())),
            ("!", Shortcut::Plain),
            ("#", Shortcut::Plain),
            ("!!", Shortcut::Plain),
            ("## Heading", Shortcut::Plain),
            ("#!/bin/sh", Shortcut::Plain),
            ("fix the bug", Shortcut::Plain),
            ("/model", Shortcut::Plain),
        ] {
            assert_eq!(parse(line), want, "{line:?}");
        }
    }

    #[test]
    fn shell_runs_in_the_folder_and_merges_stderr() {
        let d = scratch("shell");
        let r = run_shell("pwd; echo oops >&2; exit 3", &d).unwrap();
        assert!(!r.success);
        assert!(r.output.contains("oops"));
        assert!(r.output.trim_start().starts_with('/'));
        assert!(run_shell("true", &d).unwrap().success);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn long_shell_output_is_cut_and_counted() {
        let d = scratch("long");
        let r = run_shell("yes xxxxxxxxxx | head -c 30000", &d).unwrap();
        assert!(r.output.len() < MAX_SHELL_OUTPUT + 40);
        assert!(r.output.contains("more bytes"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn notes_append_as_bullets() {
        let d = scratch("notes");
        let f = d.join(".jev").join("NOTES.md");
        append_note(&f, "use  tabs\nnot spaces").unwrap();
        append_note(&f, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "- use tabs not spaces\n- second\n");
        std::fs::write(&f, "no newline").unwrap();
        append_note(&f, "third").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "no newline\n- third\n");
        let _ = std::fs::remove_dir_all(d);
    }
}
