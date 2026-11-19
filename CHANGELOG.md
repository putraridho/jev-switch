# Changelog

## Unreleased

Fixes from code review round 4.

- **Project Claude settings are trusted only as they were.** `/hooks trust` now also records
  a hash of `.claude/settings.json`; the file (hooks, `env`, helper commands) is loaded only
  while it still matches, so a later `git pull` that changes it waits for `/hooks trust`
  again. Projects trusted by an earlier version load no project settings until `/hooks trust`
  is run once more. `/hooks` shows everything in the file while it is not trusted (hook commands,
  `env`, permissions, every other key and its value), `/hooks trust` lists what it approved, and
  Claude sessions say when a trusted project's file changed and so is not loaded.
- A project's `.claude/settings.local.json` is no longer trusted as your own when git cannot
  say whether it is tracked (for instance "dubious ownership"). A folder that is not under git
  is unchanged.
- Custom commands and recipes keep `$1`..`$9` and `$ARGUMENTS` typed inside the arguments as
  typed (`/note pay $5` no longer loses the `$5`).
- `/cache` no longer lists turns that reported no token counts (Antigravity over ACP) as misses.
- Updating an extension keeps the installed copy if the new one cannot be written; `ext pack`
  does not follow links to folders (they could loop or lead out of the extension); `/diff` reads file names that git quotes (non-ASCII characters, quotes,
  backslashes); `--every` refuses an interval too large to hold; `/export` writes a private
  (0600) file.

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
