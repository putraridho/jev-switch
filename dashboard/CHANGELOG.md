# Changelog

## Unreleased

- The Chat tab is redesigned: sessions on the left, the conversation in the middle and a
  rail on the right with every provider's plan usage as a small waffle. Each reply says
  which provider took it and why in plain words, shows its tool steps on one timeline, and
  marks how much of the provider's plan window it used. Mode, model and Orchestrate move
  into the composer, and tool approvals take the composer's place until answered.
- Hovering a meter no longer pushes the page down: its detail now floats below the row on
  the dark tooltip surface, and opens with a short, springy unroll.
- The waffle answers the pointer: the hovered dot swells and a ring settles on it, then
  glides from dot to dot. The other window of the same provider fades back while you read.
- Provider headings underline and show an arrow on hover, since they open the detail view;
  buttons press down and spring back; the header's view pills darken on hover.
- Integrated **Coding chat** tab inside the main window: routes across Claude, Codex, Gemini, Grok and DeepSeek via `jev-turn`, with a built-in workspace folder browser, permission mode (`Ask`, `Edit`, `Plan`, `Auto`), model/provider pinning, live streaming tool calls, edit diffs, and interactive tool-approval cards.
- Cross-platform IO/shell fallbacks so `jev-dashboard` builds and runs on macOS in addition to Linux.
- Chat replies render minimal standard Markdown (headings, emphasis, code, lists, quotes, rules); links show their address and are not clickable. The chat opens at the repository root (the home folder when launched from Finder), keeps a stopped reply's partial text, follows new output while you are at the bottom, keeps an unsent draft across tabs, and Esc closes the folder picker first. Files the agents create get normal permissions (umask 022).

- Antigravity usage source (Gemini and Claude/GPT windows). Reset times more than 40 days
  ahead are shown as unknown.

- First version of Flashboard: overview, provider detail and settings screens; Claude, Codex
  and DeepSeek sources; history in SQLite; tray icon and threshold notifications; Waffle
  dark theme with self-hosted fonts.
- Install script with desktop entry and hicolor icons; `--uninstall` removes them and the
  autostart entry.
- Security proofs: architecture test and IPC audit.
