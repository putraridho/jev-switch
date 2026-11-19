# Changelog

## 1.0.0

Everything in 1.0.0-rc.1, plus fixes from five review rounds:

- `-p` runs obey the privacy rules, budget, failover settings and the repo's profile limit;
  their warnings go to stderr, so stdout carries only the answer.
- Privacy rules also cover the catch-up a lagging provider gets (earlier turns and git
  status), checked against the exact text that is sent.
- Custom commands, skills and recipes can narrow the mode but never widen it.
- An unreadable `.jev/team.json` holds the session to plan mode instead of lifting its limit;
  a routing rule this version cannot read no longer costs the limit.
- Hooks: input is fed on its own thread, stderr is kept (bounded) even when a background
  process holds it open, and a timeout kills everything the hook started.
- `NAME=token` secrets are redacted; `/sessions` snippets survive Unicode case folding.

## 1.0.0-rc.1

Trust, git, team and insight features for `jev-code`.

Trust
- Permission profiles: `/profile locked|careful|standard|trusted`, four named levels that map
  to the plan, ask, edit and auto modes. A repo can cap them (see Team).
- Secret redaction: `/export`, `/replay`, the audit log and the crash notice mask API keys,
  token shapes (`sk-`, `ghp_`, `AKIA`, JWTs), `NAME=value` secrets, Bearer tokens and PEM keys.
- Crash recovery: while a turn runs, `inflight-<pid>.json` in the state folder names it; if
  the process died, the next `jev-code` in that folder says which prompt was interrupted.

Git
- `/commit [hint]` and `/pr [base]` hand the agent a careful brief: stage by name, no secrets,
  no push or amend, repository message style; `/pr` branches off a trunk branch first and uses
  `gh` only when it is signed in.

Team
- `.jev/team.json`: shared routing rules (`contains` to `pin`) and `max_profile`. Rules apply
  only while no model is pinned by hand. `/team` shows them.

Insight
- Audit log (`audit.jsonl` in the state folder): one redacted line per turn, export and
  profile change. `/audit [n]`, `/history` (turns and tokens per day), `/bench` (models ranked
  by your own turns), `/replay [n]` (read a saved session back).
- `/export file.html` writes a self-contained HTML page; `.md` (the default) is Markdown.

Editors and platforms
- The editor bridge message contract (`src/bridge.rs`, `docs/bridge.md`).
- GitHub Actions CI on Linux and macOS (build, tests, PTY suite), a Windows build check.
- `docs/platforms.md`, `docs/shortcuts.md` and `docs/migrating-0.2.md`.

## 0.2.0

The interactive UI (pinned input box, `/` menu, approval cards) and the routing-aware
transcript. See the commit history.
