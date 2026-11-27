# Changelog

## Unreleased

- Hovering a meter no longer pushes the page down: its detail now floats below the row on
  the dark tooltip surface, and opens with a short, springy unroll.
- The waffle answers the pointer: the hovered dot swells and a ring settles on it, then
  glides from dot to dot. The other window of the same provider fades back while you read.
- Provider headings underline and show an arrow on hover, since they open the detail view;
  buttons press down and spring back; the header's view pills darken on hover.
- Antigravity usage source (Gemini and Claude/GPT windows). Reset times more than 40 days
  ahead are shown as unknown.

- First version of Flashboard: overview, provider detail and settings screens; Claude, Codex
  and DeepSeek sources; history in SQLite; tray icon and threshold notifications; Waffle
  dark theme with self-hosted fonts.
- Install script with desktop entry and hicolor icons; `--uninstall` removes them and the
  autostart entry.
- Security proofs: architecture test and IPC audit.
