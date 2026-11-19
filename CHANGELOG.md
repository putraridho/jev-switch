# Changelog

See [docs/changelog-guide.md](docs/changelog-guide.md) for changelog formatting, versioning rules, and release instructions.

## Unreleased

### Fixes
- **`/compact` works when the old session is full.** The summary is now requested in a fresh, read-only CLI session from a size-limited copy of the recorded conversation (tool summaries included), so it no longer depends on the old session having room left. A history over the limit keeps its beginning and its most recent part, with the omitted middle marked. If no summary comes back, the original conversation and its saved session are left intact.
- **Bounded test concurrency.** Tests default to eight harness threads instead of 24, and the 30-round Git regression uses four workers instead of spawning all 30 at once, reducing open-file pressure that could fail Git-backed tests.

### Development
- **Audit log shows what a long turn did.** Each `turn` line in `audit.jsonl` now also records `cached_tokens` (the cached share of `in_tokens`), `context_tokens` (the size of the last request, when the CLI reports it) and `tool_calls`, so a slow turn can be told apart: many tool calls over a growing context, or a cold cache. Older lines still read, with these as 0.

## 1.5.0

A terminal UI revamp (shaded prompts, light/dark themes, a btop-style side panel, live workers, quieter tool output), models that no longer promise updates they cannot give, and faster tests and reinstalls.

### Features
- **Start on a clean screen.** `jev-code` and `jev-claude` clear the terminal (screen and scrollback) when an interactive session opens, so the session owns the display. Pipes, `--no-color` and `-p` runs never get escape bytes.
- **↑ brings a queued message back.** While a turn runs, a message you sent is queued; with an empty box, ↑ takes the newest one out of the queue and puts it back in the box for editing instead of walking history first.
- **Your messages stand out.** A message you send is a full-width shaded band with a `❯` marker, and a blank line separates turns. With `NO_COLOR` or `JEV_THEME=mono` it is a plain `❯ text` under a rule.
- **Light and dark terminals.** The band shade and accent colour follow the terminal background: `JEV_THEME=light|dark`, else `COLORFGBG`, else the terminal is asked (not inside tmux or screen). Dark when none of these answer.
- **btop-style side panel.** On terminals of 100+ columns the side panel is a stack of rounded boxes with titles in the border. Plan use and context fill are colour-graded meters (`▰▰▰▱▱▱▱▱`, green, yellow, then red); a new Context box shows how full the window is.
- **Turns close with a rule.** The end-of-turn summary is set into a dim rule (`── ✓ claude · 14s ─────`), so where one turn ends and the next prompt begins is plain to see.
- **Live workers.** With orchestration on, the planner's open workers are listed in a titled box above the input box (`▶ running 0:42 · 7 tool calls`, `✓ ready to review`), refreshed about once a second. Under each running worker is its latest tool call (`⎿ Bash cargo test`), so you can see what a subagent is doing.

### Development
- **Cached test runs.** `scripts/cached-test.sh` skips a run whose inputs are unchanged since it passed (a repeat takes under a second), `changed` runs only the library test modules of the files changed since `HEAD`, and `pty [names]` caches PTY scenario runs by the built binary. Failures, and PTY filters that match no scenario, are never cached.
- **Faster tests and reinstalls.** The library suite runs in about 9s instead of 22s: its slowest test (21s alone) runs its 30 rounds in parallel, and tests use 24 threads (`.cargo/config.toml`), since most wait on git or child processes. `cargo test` no longer builds empty harnesses for binaries without tests or a doc-test pass. `scripts/install.sh` builds the new incremental `local` profile, so a reinstall after a small change takes seconds instead of a full optimised rebuild.

### Fixes
- **Long prompts no longer break mid-word.** The input box wraps at spaces; only a word wider than the box is cut.
- **What the model says between tool calls is readable.** It was dim italic; it is now full brightness, indented with the tool rows. Answers share that margin.
- **Reasoning no longer fills the scrollback.** The model's thinking is not printed between tool calls; its latest line shows on the working row (`Thinking · …`) and goes with the turn, leaving the tool rows and a short answer. `JEV_SHOW_THINKING=1` prints it as before.
- **Long runs of tool calls fold.** The first five calls of a run print as they happen; any after that are held and printed as one row (`⏺ 7 more tool calls · Bash ×5 · Read ×2`) when the run ends, with the calls that failed shown in full. Edits and delegations always print on their own. `JEV_TOOL_ROWS=all` prints every call.
- **No gap under the input box.** Beside the side panel, the router row and footer sit right under the box instead of at the bottom of the panel.
- **Models no longer promise updates they cannot give.** A model cannot speak again until you send a message, so every provider is now told to wait for its commands and never end a turn saying it will report back (Claude and DeepSeek in their system note, Codex, Gemini, Antigravity and Grok after the prompt). When a turn still left work running (a command ending in `&`, `nohup`, `setsid`, `disown`; a tool reporting `Background PIDs`; an Antigravity command still in progress) or promised a later update, a `⚠ still running` notice says to ask again.
- **No auto-allow row.** Calls allowed without asking no longer print `✓ auto-allowed` under them: the mode in the footer already says it.
- **Antigravity tool rows say what runs.** A call whose first message names only the tool (`run_command`) gets its row when its command arrives (`Bash cargo test -q`); Antigravity's `CommandLine`, `TargetFile` and `AbsolutePath` inputs are read.

## 1.4.0

Inline turn steering, delegate MCP server for Claude Code, trimmed tool hints, and security/reliability hardening.

### Features
- **Messages typed during a turn reach the running turn (Claude).** As in Claude Code, a message you queue while Claude works is taken into that turn at its next step instead of waiting for the turn to finish: `↳ message (taken into the running turn)`. A turn that ends first, and the other providers (Codex, Antigravity), keep the old behaviour: it runs as the next turn. The privacy policy and secret scanning apply to it.
- **`jev-claude` attaches the delegate tool.** Claude Code gets the MCP server `jev` (`delegate`, `show_diff`, `merge`, `discard`), allowing it to hand subtasks to background workers on your Codex and Antigravity subscriptions. Set `JEV_DELEGATE=off` to disable it.
- **Trimmed tool hints.** In auto mode, the yellow hints (`✓ auto-allowed …`, `✓ allowed (remembered) …`, denied calls) are formatted as a single line truncated to the terminal width with `…`, preventing long commands and heredocs from filling the screen.

### Hardening & Bug Fixes
- **Project Claude settings hash validation.** `/hooks trust` now records a cryptographic hash of `.claude/settings.json`. The file (hooks, `env`, helper commands) is only loaded while it matches the trusted hash. If a `git pull` or edit changes it, settings are held until re-approved with `/hooks trust`.
- **Local settings git resilience.** The local settings check no longer switches your `settings.local.json` off for harmless git failures (not under git, a locked index, or missing folders); committed files and "dubious ownership" still refuse untrusted settings automatically.
- **Extension packaging & installation safety.** `ext install` and `ext pack` refuse packages or manifests that are or traverse symbolic links. Updating an extension preserves the existing installed copy if writing the new one fails.
- **Argument preservation.** Custom commands and recipes preserve `$1`..`$9` and `$ARGUMENTS` inside arguments verbatim (e.g., `/note pay $5` no longer loses `$5`).
- **Path quoting and limits.** `/diff` reads file names that git quotes (non-ASCII characters, quotes, backslashes); `--every` enforces a safe maximum interval of 365 days; `/export` writes private (0600) files.
- **Cross-platform compilation.** Windows builds compile cleanly under `-D warnings` with proper platform gates and non-unix signal handling.

## 1.3.1

- The README is now a short introduction: what jev-router is, requirements, installation and first
  use. The full reference moved to `docs/guide.md`.

## 1.3.0

Updating from inside jev-code.

- `/update` (and `jev-code update [--yes]` in a shell) brings jev-code up to the newest commit
  of its checkout's upstream (`origin/master`): it fetches, lists the new commits and the
  version change, and after a yes fast-forwards and runs `scripts/install.sh`, its output
  shown as it runs. When the checkout is already newer than the installed program, it offers
  to reinstall without pulling.
- It never merges, rebases or discards anything: changes to tracked files (when something
  would change), a detached head, no upstream, or a branch that has diverged from it stop it,
  with what to do. Commit subjects are shown with control characters removed. The checkout is
  the one the program was built from; only `JEV_SOURCE_DIR` (environment or your own settings,
  never a project's) names another.
- It says before asking that the build cannot be stopped halfway and that installing stops a
  running jev-gateway. A running session keeps the old program until it is restarted.

## 1.2.0

Prompt suggestions.

- After a turn, the empty input box suggests the next prompt, dim, and Right puts it in the
  box. It is the step the answer ended by offering ("Want me to commit this and tag v1.1.0?"
  suggests `commit this and tag v1.1.0`), worked out locally: nothing is sent, nothing is
  spent. No offer, no suggestion; any line that runs afterwards (typed, queued, or sent with
  Esc) clears it.
- Ghost text while typing: the rest of the newest earlier message that starts with what is
  typed (or of an `@file`) shows dim after the cursor, and Right takes it. It stays out of the
  way of menus, approval cards, history browsing and multi-line text.

## 1.1.0

Security fixes from an audit of 1.0.0. Some defaults change; see "Behaviour changes".

Behaviour changes
- A repo's committed `.claude/settings.json` is loaded by Claude Code only after `/hooks trust`;
  until then only your own settings and an untracked `.claude/settings.local.json` are read
  (`--setting-sources`). jev-code's own "don't ask again" check ignores a committed
  `settings.local.json` too.
- Hook trust is bound to the hooks file's SHA-256: any change to `.jev/hooks.json` turns the
  project's hooks off until it is trusted again. `/hooks` and `/hooks trust` show the commands.
- Your own commands and extensions win over a project's command of the same name.
- A project's `.jev/config.env` may turn `JEV_PROVIDERS` and `JEV_ORCHESTRATE` off, never on.
- `CODE_PERMISSION_MODE` only picks the starting mode; it is never passed to Claude Code as
  given, so `bypassPermissions` is ignored, and `-p` runs follow the session's mode.

Permissions and modes
- Orchestration never runs in plan mode; its `merge` tool is approved without asking only in
  edit and auto (Claude and Codex).
- Gemini CLI and Grok auto-accept edits only in edit and auto; in ask and plan they make none.
- A shared `team.json` pin is skipped when your own routing rules avoid that provider.
- Saved rules never cover `--kubeconfig` or Go's `-toolexec`, `-vettool` and `-exec`.

Privacy
- The privacy policy holds with routing off and for orchestration workers; briefs are masked.
- Jev's classifier gets prompts with secrets masked, and is not asked when the policy keeps a
  prompt on this machine.
- The redactor catches `xai-`, Stripe, GitLab, Hugging Face, npm and Google OAuth keys,
  passwords in URLs, Basic auth, `x-api-key`, `*_KEY=` and escaped JSON; the audit/export
  redactor uses the same detection. `/sessions export` masks secrets.
- Privacy paths: trailing punctuation no longer hides a path, globs ignore case, and an
  unreadable `privacy.rules` turns local-only on.

Terminal and files
- Command output, edit previews and model reasoning have escape sequences and control
  characters removed (no clipboard, link or title injection).
- The state folder is 0700, the probe cache 0600, atomic writes refuse symlinks, and the `/tmp`
  prompt folder is checked for owner and permissions.
- jev-code's own git calls turn off `core.fsmonitor`.

Gateway
- 403 for requests with an `Origin` header or a non-local `Host`; requests without credentials
  are not routed; at most `JEV_GATEWAY_MAX_INFLIGHT` (32) at once; the session map is capped; a
  non-loopback `JEV_GATEWAY_ADDR` needs `JEV_GATEWAY_ALLOW_REMOTE=1`.

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
