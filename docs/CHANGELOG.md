## 1.4.0

- Queued input during a turn reaches the running turn (inline steering for Claude).
- Claude Code gets the `jev` delegate MCP server to route tasks to background workers.
- Trimmed tool hints in auto mode keep long commands to a single line.
- Project settings hash validation: `/hooks trust` verifies `.claude/settings.json` before loading.
- /help takes a search: `/help diff` lists matching commands and keys.
- /tips lists short tips; /keys (or `?` on an empty box) shows key cheat sheet.
- /whatsnew shows what changed in this version (`/whatsnew all` for all versions).
- Vim input mode (`JEV_VIM=1`): Esc for normal mode, then h j k l w b 0 $ x dd D i a I A o.
- `@` finds files by scattered letters when no name starts with what was typed.
- Reduced motion (`JEV_REDUCED_MOTION=1`) keeps spinner still; `JEV_THEME=mono` turns colour off.

## 0.2.0

- New interactive UI: an input box pinned under the conversation, a / menu, numbered approval cards, a footer with the mode.
- A transcript built for routing: a route row leads each turn, a router row shows plan use and cooldowns, delegated pieces show as Delegate rows.
- /rename names a session, /resume lists this folder's sessions to pick from, /mcp shows MCP servers.
- JEV_CLASSIC_UI=1 keeps the old interface.
