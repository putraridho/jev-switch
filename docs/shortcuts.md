# Shortcut reference

`/help` shows the same table; it is generated from one list in `src/uifmt/commands.rs`.

| Key | What it does |
| --- | --- |
| Enter | Send; while a turn runs, queue it for after the turn |
| Shift-Enter | New line inside the message (also Alt-Enter, Ctrl-J, or `\` then Enter; `/terminal-setup` makes Shift-Enter work in your terminal) |
| `/` | Command menu: Up/Down choose, Tab complete, Enter run, Esc close |
| `@` | Pick a file or folder to point the agent at |
| Esc | Stop the running turn; what you typed is sent right after |
| Ctrl-C | Stop the running turn and drop the queue; clears the box; twice on an empty box exits |
| Ctrl-D | Exit (empty box) |
| Up / Down | Earlier and later messages |
| Ctrl-R | Search earlier messages |
| Ctrl-L | Clear the screen |
| Shift-Tab | Cycle the mode: ask, edit, plan, auto |
| `!command` | Run in the shell; the output goes with your next message |

## Commands added in 1.0.0-rc.1

`/profile [name]`, `/commit [hint]`, `/pr [base]`, `/team`, `/audit [n]`, `/history`,
`/bench`, `/replay [n]`.
