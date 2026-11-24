# Platforms

| Platform | Status                                                                                                                                                                                                                             |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS    | Developed and tested here: build, unit tests and the PTY suite.                                                                                                                                                                    |
| Linux    | Same code paths as macOS; built, tested and run through the PTY suite in CI.                                                                                                                                                       |
| Windows  | Native, **experimental**: it builds and passes clippy in CI (a hard check), and releases include `windows-x86_64`. It has not been run by a person on a Windows machine yet, and the unit tests run advisory in CI (some still assume `sh`). See "Windows" below. |

Notes

- Needs `git` on the PATH, and at least one provider CLI (`claude`, `codex`, `gemini`, `grok`).
- `/pr` uses the GitHub CLI `gh` only if it is installed and signed in; otherwise it prints the
  title and body.
- Shift-Enter needs terminal help: run `/terminal-setup`.

## Windows

What works by design (and is checked by compiling for Windows, not yet by using it): the
terminal reads keys through the Windows console (approval questions, typing ahead during a turn,
`/resume` and `/mcp` pickers, terminal size), `!command` and hooks run through `cmd /C`, a
turn's process tree is stopped with `taskkill /T`, `HOME` falls back to `USERPROFILE`, provider
CLIs are found as `name.exe` / `name.cmd`, and `/update` replaces a running program by renaming
it aside.

Install: `irm https://raw.githubusercontent.com/putraridho/jev-switch/main/scripts/install-release.ps1 | iex`.
It puts `jev-code.exe`, `jev-claude.cmd` and the rest in `%USERPROFILE%\.local\bin`; settings,
state and the orchestrate mod use the same `.local` folders as on Unix, under `%USERPROFILE%`.

Known gaps:

- Commands you give to `!`, hooks and `/orchestrate` checks are `cmd` commands, not `sh`.
- `jev-claude` is a PowerShell script (`jev-claude.cmd` starts it); `scripts/install.sh` and
  updating a source checkout are Unix only.
- Bracketed paste is not available in the console, so a multi-line paste while typing ahead
  arrives as separate keys (Enter inside it is kept as a new line when it arrives in one batch).
- Whether each provider CLI (`claude`, `codex`, `grok`, `gemini`) behaves the same on Windows is
  not verified. Run `jev-code doctor` first.
- The PTY test suite does not run on Windows.
