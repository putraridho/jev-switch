# Platforms

| Platform | Status |
| --- | --- |
| macOS | Developed and tested here: build, unit tests and the PTY suite. |
| Linux | Same code paths as macOS; built, tested and run through the PTY suite in CI. |
| Windows | Not supported for running `jev-code` yet. CI only checks that it builds (allowed to fail). The terminal layer is Unix based (SIGINT streams, a pty test harness), and crash recovery treats every process as alive there. Use WSL. |

Notes
- Needs `git` on the PATH, and at least one provider CLI (`claude`, `codex`, `gemini`, `grok`).
- `/pr` uses the GitHub CLI `gh` only if it is installed and signed in; otherwise it prints the
  title and body.
- Shift-Enter needs terminal help: run `/terminal-setup`.
