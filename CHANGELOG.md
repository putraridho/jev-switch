# Changelog

See [docs/changelog-guide.md](docs/changelog-guide.md) for changelog formatting, versioning rules, and release instructions.

## Unreleased

- **The mouse wheel scrolls the chat in full-screen mode, and text selection still works.** With the mouse left to the terminal, most terminals send the wheel as Up and Down arrow keys, which recalled history or moved the cursor instead of scrolling. A burst of arrows that arrive together in one direction (one wheel notch) now scrolls the chat one row per arrow, while a single arrow press keeps its meaning. The terminal reports nothing, so this is a heuristic: a wheel that sends one arrow per notch still looks like a key press, and the arrows go to a completion menu or an approval question when one is open. Use PageUp/PageDown, or `JEV_INLINE=1` for the terminal's own scrollback, in those cases.

## 1.14.0

Downloadable releases, `/update` that installs them, and experimental native Windows.

- **Releases are downloadable binaries, and `/update` updates them.** Pushing a `vX.Y.Z` tag now builds jev-code for Linux and macOS (x86_64 and arm64) and publishes `jev-code-vX.Y.Z-<platform>.tar.gz` with its SHA-256 on GitHub. A downloaded jev-code runs `/update` (or `jev-code update [--yes]`) against the newest release: it shows the version change, and after you confirm downloads the tarball, verifies the checksum and replaces `jev-code`, `jev-ask`, `jev-chat`, `jev-gateway`, `jev-delegate-mcp`, `jev-claude` and the orchestrate mod. Every file is staged before any is replaced, so a failed download, a checksum mismatch or an unwritable folder changes nothing. A build from a checkout still updates the checkout; `JEV_SOURCE_DIR` always selects that. `JEV_INSTALL_DIR` names another install folder. The once-a-day startup notice and `JEV_UPDATE_CHECK=off` work the same for both.
- **Native Windows support (experimental).** jev-code now builds and passes clippy on Windows in CI (a hard check; the old Windows job failed on `libc` and was allowed to), and releases include `jev-code-vX.Y.Z-windows-x86_64.zip`. On the Windows console, approval questions read keys (they used to answer "no" without asking), typing ahead during a turn, `/resume` and `/mcp` work, and the terminal width is read from the console. `!command`, hooks and checks run through `cmd /C`, a cancelled turn's process tree is stopped with `taskkill /T`, `HOME` falls back to `USERPROFILE`, and provider CLIs are found as `.exe` or `.cmd`. `/update` replaces running programs by renaming them aside. `scripts/install-release.ps1` installs a release and `jev-claude.cmd` starts the new `jev-claude.ps1`. Nobody has used it on a real Windows machine yet; see `docs/platforms.md` for what is verified and the known gaps.
- **Update checksums no longer need `sha256sum` or `shasum`.** The release updater checks the SHA-256 itself.
- **Install without building.** `scripts/install-release.sh` (`curl … | sh`) downloads the newest release for your platform, verifies it and installs it into `~/.local/bin`.

## 1.13.3

Queued messages stay in sync with the running turn.

- **Keep queued messages in sync with the running turn.** Up checks pending provider updates before taking a queued message back for editing. Ctrl-C also withdraws dropped messages from the provider, so they cannot be sent after disappearing from the queue.

## 1.13.2

A thread now names itself, so the terminal tab shows it.

- **A new thread is named from its first message.** The terminal tab and header showed `jev-code` until you ran `/rename`, because nothing named a thread on its own. An unnamed thread now takes a short title from the message you send (slash commands do not name it), and the tab shows it as soon as the turn starts. `/rename` still overrides it.

## 1.13.1

Terminal selection, readable reasoning lines and thread-named tabs.

- **Mouse stays with the terminal in full-screen mode.** `jev-code` no longer enables terminal mouse tracking, so clicking and dragging selects text (including block selection) and clicks links as usual in your terminal; use PageUp/PageDown and Ctrl-Home/Ctrl-End to scroll the chat pane.
- **Thought lines wrap instead of truncating.** Long `◆ Thought` lines now wrap onto indented continuation rows aligned under the first word rather than cutting off with `…`.
- **Terminal tabs show the thread name and state.** The window and tab title now shows the activity mark (`○`, `●`, `?`, `✓`, `✗`) followed directly by the session name (or `jev-code` when unnamed).

## 1.13.0

True billing, one shared budget, measured quality and provider health.

- **Context windows and prompt caches are now facts about one model on one backend, with their evidence shown.** A context window a backend reports (Codex's model cache and the window in a thread's rollout, the window Claude Code and DeepSeek print for a one-model turn, an ACP agent's usage size) replaces the catalog's fixed size for that backend only, and absurd values are ignored. Models are filtered on the session plus what the request really adds in the receiving provider's tokens: the new prompt, catch-up text, tool definitions and room for the answer, with a characters-per-token rate measured per provider only from turns that reported a request size. After an answer only the cache the provider reported (read plus written) counts as warm, not the whole context; the cache window is a prior flagged "assumed" until observed gaps (a hit after a gap, a miss after a gap) move it, a provider with no documented window is no longer warm for ever, and the cache is identified by provider, model and backend. Counted hit ratios are kept between sessions (counts only, no prompt text, 30 days) and `/bench` shows them with the number of turns and `assumed`/`observed` labels. A warm cache stays advisory: it never overrides the quality floor, a hard rule, plan steering or a pin, and a cache that counted turns show rarely hits no longer holds a model.
- **Providers are proven rung by rung, and a known incompatibility is no longer rediscovered every session.** Each provider now has a readiness ladder (installed, logged in, entitled, protocol-compatible, validated for this project) with evidence and time per rung; rungs nothing has shown stay "unverified". A lasting failure (a login that failed, a retired client, a folder the CLI refuses) holds the provider until the underlying fact changes (the CLI file, the login file, the funding backend, the folder's trust) or 24 hours pass, and is remembered across sessions in `provider-readiness.json`; a model your plan lacks is remembered per project. Empty answers and stalls keep the circuit breaker, which now lets exactly one trial attempt through after its pause (an answer closes it, a failure reopens it for twice as long). `jev-code doctor` and the new `/providers probe` run a capability handshake that sends no prompt and spends no quota (`--version`, the flags jev-code relies on in `--help`, an ACP `initialize` for `agy` and `grok`, folder trust). `/providers` shows the rung reached, the last evidence time and the next retry; `doctor` explains how to fix each failed rung; a pinned provider that is held gets an error naming the rung instead of being swapped.
- **Accepted work is measured, shown and can steer routing.** A turn is now judged accepted, rejected or unverified from what is observable: only a passing check after changes accepts it, a failed check or an undone-then-redone turn rejects it, and an answer alone is never "accepted". `/verify off|auto|<command>` (also `JEV_VERIFY`, default off) runs a check after a turn that changed files: `auto` picks the project's cheapest (`cargo check`, an npm `test` script, `pytest -x`, `go build`), never in plan mode or after a turn that changed nothing, with a 5-minute timeout and truncated output. A failed check is put in front of your next prompt once. `JEV_VERIFY=on` now means `auto` (it used to run the full test command). The audit log records the task type, acceptance, check time, retries, rework and tokens per turn (older lines still read). `/bench` shows accepted-rate with `n=`, rework and tokens per accepted turn per model and task type. With at least 5 checked turns each, models of the same tier reorder by accepted-rate then cost per accepted turn (`JEV_LEARNED_ROUTING=off` disables it); pins, your chains, security, budget and quota steering are never overridden and no data changes nothing. `scripts/provider_eval.py audit --arm LABEL=PATH ...` compares a router run with a pinned baseline from audit logs alone.
- **One usage ledger and budget admission for the planner and its workers.** Before an attempt starts, jev-code now reserves its estimated maximum in a ledger file shared by the main process and the `delegate_mcp` processes (`usage-ledger.json` in the state folder, changed under a lock), and settles the reservation with what the attempt really used. Planner turns, retries and failovers, and every worker attempt are admitted the same way, so concurrent attempts can no longer pass a paid cap together, and the planner's spend is seen by its workers across processes. Failed or cancelled attempts without a report count as unknown consumption, and the reservation of a process that died is reaped as unknown too. New: a daily paid cap over all sessions (`/budget daily usd 20`) and a soft subscription allowance per provider (`/budget allowance 5m`, a warning, never a refusal). `/budget` now shows paid money (hard) and subscription quota (soft) apart, with what is reserved in flight.
- **Billing follows how each CLI is actually funded, not the catalog's label.** jev-code now reads each provider's own configuration (presence and kind only, never a key): Codex's `model_provider` and sign-in mode, a Grok, Gemini or Claude API key against a saved login, a custom endpoint, and DeepSeek's key. A CLI that is not on its native login (an API key, a custom endpoint such as a non-OpenAI Codex provider, or no login that can be seen) is treated as paid use: the budget cap, the paid-fallback switch and the worker checks apply to it, and `/budget` shows "billing unknown" instead of inventing dollars. `/providers` and `jev-code doctor` show the kind per provider ("ChatGPT login", "API key (per token)", "custom endpoint (billing unknown)").

## 1.12.0

Accepted work, steadier providers and a shared budget.

- **Antigravity and Grok token counts are used when the agent reports them.** If the finished prompt carries input and output token counts (and the cached share), the turn's consumption is recorded instead of staying unknown. An agent that reports nothing, or only half of it, still shows as unknown, never as zero.

- **The paid-use cap counts the workers too.** The planner and the delegate tool's workers (a separate process) now write each finished attempt to a shared `spend-ledger.jsonl`, and both sides check the cap against the total of the session: a worker can no longer spend after the planner would refuse the same paid model, and `/budget` shows the combined spend. A worker counts from its project's newest session start; spend with no reported usage still makes a capped session fail closed. This is still an admission check between attempts, not a hold on spending already under way.

- **Codex models use the context window your backend reports.** jev-code reads Codex's own model list (`$CODEX_HOME/models_cache.json`, the usable share of each window) instead of assuming 500k for every Codex model, so a model with a larger window is no longer skipped for a big session, and a smaller one is no longer overfilled. `/context` shows the same window. Models Codex does not list keep the built-in size.

- **A provider that keeps failing is paused instead of rediscovered every session.** After four empty answers, stalls or retired-CLI errors in a row, a provider is skipped for 10 minutes (doubling each time it fails again right after, up to 2 hours), and the pause is remembered across sessions and by workers. Any answer ends it, `/providers refresh` ends it at once, and `/providers` says why a provider is paused. Limits, logins, folder refusals and unknown errors never count.

- **`JEV_VERIFY=on` checks a turn's work with your project's own tests.** After a turn that answered and changed files, jev-code runs the project's test command (the same one workers use), says whether it passed, and `/health` shows how many of each provider's turns passed. The verdict is also kept in the audit log, and `/bench` shows `tests passed N/M` per model, so you can compare models by work that held up, not by answers returned. Off by default; a run with no test command, a timeout or a Ctrl-C counts as not checked, never as a failure.

## 1.11.13

Update notices.

- **jev-code tells you when a newer version is available.** At the start of an interactive session it checks the source checkout's upstream in the background (at most once a day, never delaying the banner) and prints `jev-code X is available (you have Y): run /update` when the version on main is higher. A version found earlier is announced again at each start until you install it. `JEV_UPDATE_CHECK=off` turns the check off.

## 1.11.12

Grok bills your API key, not your login's plan.

- **Grok uses your `XAI_API_KEY` even when you are also signed in with `grok login`.** The Grok CLI prefers a saved login over the key, so turns ran on the login's plan and stopped at its free usage limit. With a key set, jev-code now runs Grok under a Grok home that links everything in `~/.grok` except the login (`~/.local/state/jev-router/grok-key-home`). Sessions, config, skills and memory stay shared, and your login is left untouched for plain `grok`.

## 1.11.11

Routing safeguards, honest usage accounting, bounded handoffs, and budget-aware workers.

- **The example environment file lists the missing credentials and settings.** It includes the Grok API key, the Jev key alias, compaction, worker auto-merge, privacy, log paths, and tuning controls. Shell-only UI, gateway, and evaluation settings are separated from file-backed settings, and the orchestration defaults are corrected.
- **Hard security constraints survive routing-service failures.** A missing judgment no longer lets security-sensitive requests remain on DeepSeek or Grok; explicit pins are still refused rather than silently replaced when barred for the task.
- **Continuing difficult work can upgrade without losing its provider unnecessarily.** Automatic routing meets the task's quality floor before retaining a light model, and a nearly exhausted Claude plan can move continuing work to an eligible alternative. User pins remain unchanged.
- **Context occupancy is no longer guessed from cumulative turn consumption.** Missing per-request measurements show as unknown and do not trigger false context-overflow errors or automatic compaction.
- **Failover carries partial work forward.** Every failed attempt's file and command activity is recorded before retrying and stays in the saved conversation after another provider succeeds; discarded draft answers are not passed on. Tool counts include all attempts.
- **Provider handoffs respect their estimated token budget.** Long histories keep a bounded recent excerpt, identify omitted earlier turns, and leave the canonical history intact.
- **Unknown consumption is not reported as free usage.** Antigravity and ACP workers report unavailable consumed-token counters explicitly; JSON uses null rather than zero. Known consumption from empty or quota-ended results is retained, and failed or cancelled attempts without counters are marked unknown in the meter. Paid caps stop further paid admission when consumption cannot be checked.
- **Delegated workers enforce paid-use policy at model admission.** They honor reached caps and paid-fallback restrictions, and completed attempts feed the same synchronized worker-policy meter. This is still policy-instance accounting, not a shared ledger across the main agent and MCP processes or a reservation for concurrent work.

## 1.11.10

Failover moves past providers that stop answering, background gateway requests back off after rate limits, and fake CLI tests keep your failure logs clean.

- **An empty or stalled provider no longer consumes the whole failover chain.** After two different models from one provider fail to answer, the remaining models from that provider are skipped for the current turn. Other providers can still answer, and the next turn can retry without a persisted cooldown.
- **Fake CLI tests no longer write apparent crashes and usage-limit bursts into your failure log.** Scripted test backends disable failure recording for their own commands, including when another crate runs them. Real provider diagnostics remain enabled, and command-local `JEV_FAILURE_LOG` overrides can direct a run to an isolated log.
- **Background gateway requests stop hammering models that just returned 429.** One-shot auxiliary requests remember each rejected model across turns, use Sonnet when Haiku is cooling down, and return a temporary 429 with `Retry-After` when both are unavailable. Concurrent requests share the same bounded fallback; main turns, tool-using forks, subagents and explicitly requested models are unaffected.

## 1.11.9

Codex commands can use the network and local ports when you allow it, and say so when the sandbox blocks one.

- **`CODEX_NETWORK=on` lets Codex commands use the network and local ports.** Codex runs in a `workspace-write` sandbox with no network, so `vitest`, a dev server, `gh` or `git push` failed inside it with `listen EPERM: operation not permitted 127.0.0.1` or "GitHub access is blocked", and its summary then covered only what it could reach. The setting passes `sandbox_workspace_write.network_access=true`; file writes stay confined to the workspace, and plan mode stays read-only and closed. It is off by default.
- **A Codex command the sandbox blocked now says so.** The first failed command per turn that looks like a blocked port or network call shows a hint naming `CODEX_NETWORK=on` (or another model) as the way out.

## 1.11.8

A Grok run that dies with `cancelled` fails over instead of ending the turn, and failed CLI lines are kept for diagnosis.

- **A Grok run that reports `cancelled` on its own moves to the next model.** Grok sometimes ended a turn with the bare word `cancelled` (13s and 80s in, no key pressed). It was classified as an ordinary failure, so the turn stopped with no answer and had to be sent again. It is now treated like a stalled run: the turn fails over, with a note that the model stopped answering. Your own Ctrl-C is unaffected, and a longer message that merely contains the word is still an ordinary failure.
- **The raw line behind a failed CLI attempt is kept.** The turn log only holds the message a backend extracts, which can be one word. Every failure line a provider CLI prints is now also appended, with secrets masked and cut at 4000 characters, to `provider-failures.jsonl` in the state folder (`JEV_FAILURE_LOG` moves it), so the cause of the next one can be read.

## 1.11.7

- **Paste copied images and files with Ctrl-V on Linux.** The interactive prompt now reads PNG images, file-manager selections and text from the Wayland or X11 clipboard (requires `wl-clipboard` or `xclip`). Images are saved privately and attached to the next message; clipboard failures appear in the prompt.

## 1.11.6

Gemini and Grok workers can run shell commands again.

- **Workers answer tool calls by a rule instead of denying them all.** A delegated worker has nobody to approve its tool calls, so every shell command a Gemini worker asked about was denied and the worker stopped without an answer (9 of 13 Gemini delegations failed). A worker now may run what `CODE_ALLOWED_TOOLS` allows (editing workers only) and commands that only read or build and test the project (`git status`/`diff`/`log`, `grep`, `cat`, `cargo test`, `npm test`, …), as long as no path in them leaves its worktree (`/…`, `~`, `..`). Committing, the network and anything else is still denied. Claude Code and Codex workers are unchanged: they decide inside their own CLI.
- **Grok workers run over ACP.** Headless Grok denied every shell command itself; a worker now runs `grok agent stdio`, so its shell commands reach the same rule, and an editing worker may edit files inside its worktree (not `.git`), since Grok has no accept-edits mode. Grok turns in a session are unchanged. The request shapes were taken from the ACP spec; anything Grok sends in another shape is denied, never allowed.
- **A Grok model the login cannot use is skipped.** "unknown model id" (as `grok-build-0.1` is for a free grok.com login) now counts as not on the plan, so the next model is tried, and a Grok quota error over ACP keeps its detail so the provider cools down.

## 1.11.5

A message that starts with `#` reaches the model instead of landing in AGENTS.md.

- **Only `# note` (a space after the `#`, one line) is a note.** Any message that started with `#` was appended to the project's AGENTS.md and never sent, so `#995 is merged already`, `#2, I allow` or a pasted block beginning with `#` vanished into the team's instructions file. A note now needs a space after the `#` and fits on one line; everything else is sent as a message.

## 1.11.4

Gemini 4 Neon joins the model catalog.

- **Gemini 4 Neon (`gemini-4-neon`) is routable.** Google's new frontier model is listed as a Top-tier Gemini model, available on the Google AI Pro/Ultra subscription. It is now the Gemini pick for security, hard debugging and review turns, ahead of Gemini 4 Argon, which stays as the next Gemini fallback. Like Argon, it is not reserved for very hard turns only. The catalog was re-checked against every provider on 2026-11-21; no other current, generally available model was missing.

## 1.11.3

A prompt with a dash or another multibyte character before a secret-looking name no longer crashes the turn.

- **Redaction no longer panics inside a multibyte character.** Every outgoing prompt is scanned for secrets. The scan took the character before `:` or `=` by byte and stepped one byte past it, so a multibyte character there (an en dash, an arrow, an emoji) put the slice inside that character and the turn panicked. A prompt as ordinary as `Keep PKCE – : mandatory` was enough. The scan now steps past the whole character.

## 1.11.2

Resuming a session brings its conversation back onto the screen.

- **`/resume` and `-c` bring the old conversation back onto the screen.** Resuming a session only printed a line such as `resumed 12 turns`, so the scrollback stayed empty even though the session was loaded. It now draws the saved turns as they looked when they ran: what you sent, the model that answered, its tool calls and the answer. A session of more than 30 turns shows the latest 30 and says how many earlier ones are not shown.

## 1.11.1

A long pasted prompt is routed on what you asked, not on the paste.

- **A long prompt with a big paste no longer routes as `unclear`.** The whole prompt, including a page of pasted HTML, went to Jev's judgment, so the paste drowned the ask and the turn was classified `unclear (unsure)` and sent to a mid-tier model, which also kept subagent orchestration from starting. The judgment now reads the start and the end of a long prompt (1,500 characters each, with a note between), where the ask usually is. The model still receives the whole prompt.

## 1.11.0

Cross-provider subagents and delegation telemetry in jev-code, Gemini Antigravity (`agy`) integration, and queued message withdrawal fix.

- **Subagents and delegation work seamlessly across all providers in `jev-code`.** Subagent and worker invocations across Claude (`Agent`, `Task`), Codex (`spawn_agent`, MCP tool calls), Antigravity/Gemini (`invoke_subagent`, `subagent`), and Jev's cross-provider orchestrator (`delegate`, `jev__delegate`, `mcp__jev__delegate`) now consistently display with the dedicated cyan `⇉ Delegate` marker in the transcript, stand alone instead of getting grouped into collapsed tool runs, and cleanly report active delegation on the working spinner. Codex tool calls, web searches, and subagent items are now streamed and completed live.
- **Gemini runs on Antigravity (`agy`) whenever it is installed.** Google retired Gemini CLI's own login, yet jev-code kept choosing `gemini` when it found that old login, so a Gemini turn failed and fell over to another provider. Now `agy` is used whenever it is on PATH, and `gemini` only on a machine without `agy`. Gemini counts as ready only when `agy` is signed in (or an API key is set), and the install, login and trust hints name `agy`.
- **A queued message you take back to edit is no longer sent twice.** While a turn runs, Enter queues your message and offers it to the running turn. Pressing ↑ to take it back out and edit it used to leave that first copy with the turn, so the turn received both the original and your edited message. Taking a message back now withdraws the copy the turn was offered, and only the edited message is sent.

## 1.10.0

`/mcp` becomes a list you can navigate: one row per server, live status, a card per server, and login from the list.

- **`/mcp` is a list you can move around in.** Each server shows once, however many providers have it, with a status dot that fills in while you look: `●` connected, `◐` needs login, `✗` failed, `○` off, `·` not checked. The dots come from `claude mcp list` (which connects to each server) and `codex mcp list --json` (which says whether a sign-in is missing), run in the background. ↑↓ move, typing narrows the list (by name, URL, transport or provider), Enter or → opens a server's card with its full target, how each provider sees it (and why one failed), and a `Log in with claude` / `Log in with codex` action. ← or Esc goes back; Esc clears a search before it closes the list.
- **`/mcp login` without a name opens the same list**, and Tab completes server names after `/mcp login`. `/mcp list` still prints the plain table, as does `/mcp` when there is no terminal.

## 1.9.3

Bug fix: the context meter no longer reads full after one busy Codex turn.

- **The context meter is right after a Codex turn.** Codex reports one token count per turn, the sum over every request the turn made, and an agent turn re-sends the conversation on each tool call. jev-code took that sum as the context size, so a short session with one busy Codex turn showed 100% (878K "used" when the conversation was 51K), warned that switching away would rebuild 878K tokens, and weighed staying with Codex on that inflated cache. It now reads the last request's size from Codex's session file (`~/.codex/sessions/…/rollout-…-<thread>.jsonl`, or under `CODEX_HOME`), and falls back to the turn total only when that file has none.

## 1.9.2

Bug fix: `/update` works again when a local tag differs from the one on GitHub.

- **`/update` no longer fails on a clashing tag.** It fetched with `--tags`, so one local tag that differed from the remote one (`! [rejected] v1.0.0 -> v1.0.0 (would clobber existing tag)`) failed the whole check with only "failed" to show. It now does a plain fetch, which still picks up new release tags but leaves your existing tags alone.
- **Update failures show Git's diagnostics and exit status.** `/update` keeps messages from Git and its transport helpers, including helpers that write to standard output, so fetch failures are easier to diagnose.
- **`/update`'s help says `main`.** It still named `master`, the branch's old name; the update follows `origin/main`.

## 1.9.1

Bug fixes: a turn now moves on from a Gemini CLI that Google retired, and a failed tool shows a clean message.

- **A retired Gemini CLI no longer ends the turn.** When Gemini answers "This client is no longer supported … migrate to Antigravity", the turn fails over to the next model, as it does for a usage limit, and gemini is skipped for the rest of the session. `/providers`, `/doctor` and the scoreboard say why and what to do (install Antigravity, `agy`, and log in there). `/new` does not clear it, since only a new CLI can.
- **Claude is no longer sent to tools it does not have.** The note jev-code gives claude told it to use `Grep` and `Glob`, which current Claude Code builds do not offer, so it called them, failed, and fell back to `sed`. The note now says to search with them if it has them, else `grep` or `find`.
- **A failed tool shows its message without Claude Code's `<tool_use_error>` tags.**

## 1.9.0

A new look for jev-code, modelled on Grok Build: only the input box is boxed, and the conversation gets the room.

- A header row on top names the branch, the folder and the session, and on the right how turns are routed, each subscription's plan use and how full the context window is. It replaces the side panel and the router row under the box.
- A run of tool calls prints as one row that reads as a sentence, `◇ Edited parser.rs, explored 3 files, ran 2 commands +12 −4`, once the run is over. A call that failed still prints its own rows under it, with the output that says why. While the run goes, the working row says what is running now. `/diff` shows the edits line by line; `JEV_TOOL_ROWS=all` brings back a row, a result and a diff for every call.
- What you sent is a padded shaded band with the time you sent it. The model's thinking leaves `◆ Thought · …`, the answer is plain bright text (no bar down its edge), and a turn ends with a quiet `Worked for 14s · ✓ codex`.
- The input box carries the route and the permission mode in its bottom edge (`auto · edit mode`). Under it, the keys that work right now: `Enter:send │ Shift+Enter:newline │ Shift+Tab:mode │ /:commands │ @:files`, or the menu's and a running turn's keys. A hint that does not fit is left off whole.
- The `/` menu opens above the box. It lists command names only, so descriptions fit on narrow terminals; the selected command's arguments (`/model [target]`) head the menu, and the selected row is raised.
- An approval or a yes/no question is a raised card with a bar down its left edge and `● 1. Yes` / `○ 2. …` choices.
- Messages queued during a turn show as `#1 then run clippy`, and the working row reads `⠹ Running cargo test 12s`, the command in the accent colour.
- Text stays readable on light terminals. Yellow, green, cyan and magenta (warnings, the mode name, code spans, provider names) use darker shades there that measure at least 4.5:1 on white; before, some measured under 2:1. Dark terminals keep their own palette.
- The input box's border is easier to see, so it is clear where to type.
- The header and the banner shorten a long folder path from the front (`…/project/repo`), keeping the folder name instead of cutting it off.
- A failed turn's message is indented like the rest of the turn and names the model as `claude · sonnet-5-5`.

- **Code layout (no change in behaviour).** The source is now organised as bounded contexts (`approval`, `backend`, `catalog`, `changes`, `judgment`, `orchestration`, `providers`, `routing`, `session`, `shared`, `turn`, `tui` and more), each split into `domain`, `application` and `infrastructure`. An architecture test checks which context may import which and that an inner layer never reaches outward; `docs/architecture/DDD.md` has the plan. Every command's output is byte-identical to 1.8.2 apart from the UI work above.

## 1.8.2

The repository's main branch is renamed.

- The repository's main branch is now `main` instead of `master`. A checkout that updates with `/update` keeps working as long as its upstream is set; if yours still tracks `origin/master`, run `git branch -m master main && git fetch origin && git branch -u origin/main main`.

## 1.8.1

Antigravity follows `CODE_ALLOWED_TOOLS`, and a denied command no longer burns through every model.

- `CODE_ALLOWED_TOOLS` now also covers Antigravity, which asks jev-code about each command instead of reading the rules itself. A command your rules allow (for example `Bash(python3 *)`) runs without a question, also in `-p` and scheduled runs, and the screen shows `✓ allowed (CODE_ALLOWED_TOOLS)`. Rules apply in every mode except plan. Each command in a chain must match, and commands with `$`, backticks, subshells or redirects to a file never match.
- When a run has nobody to approve a command and the model then stops with an empty answer, the turn now ends with one message saying which tool was denied and how to allow it. Before, jev-code counted it as a failed model and tried the same prompt on other models, which all hit the same denial.
- jev-code's own git calls no longer run filter programs set in the repo's own `.git/config` (or its `config.worktree`). An agent edit to that file can no longer make jev-code run a command without asking. git-lfs and git-crypt filters still run when they point at the `git-lfs` or `git-crypt` on your `PATH`, and so do filters in your global git config.
- A `.jev/team.json` pin that fails during a turn now hands the turn to automatic routing instead of ending it. The route line says the pin was skipped. A pin that cannot be used before the turn starts is still reported, as before.
- A shell command that prints a very long line no longer shows an extra empty line after it.
- Files the system adds on its own (`.DS_Store`, `._*`, `Thumbs.db`, `desktop.ini`) no longer stop jev from replacing an extension it installed.

## 1.8.0

Delegation that does not waste the planner's time or tokens, and plan usage from the start.

- `delegate` no longer makes the planner sit idle: it returns a task id at once, and the worker runs and is tested in the background while the planner keeps working. A new `wait` tool collects the results, and auto-merges happen there, never while the planner may be editing. `wait` takes optional task ids and a timeout (`0` only checks).
- Delegated work merges itself when it is proven: when an editing worker finishes, jev runs the project's tests in its worktree, and if they pass and the changes apply cleanly they go into your working tree when the planner collects the result. The planner gets the report and the file list instead of reading the whole diff, which saves tokens on every worker. A new `undo` tool takes one worker's changes back out. With no test command, failing tests, a conflict, or `review: true`, the planner reviews the diff as before. `JEV_AUTO_MERGE=0` turns auto-merge off.
- Read-only delegated pieces (research, finding code) now run in your project folder in plan mode with no worktree, so they see your uncommitted edits and leave nothing to merge. On Gemini and Grok, whose CLIs do not enforce plan mode, they still get a throwaway worktree.
- Worker log records carry `started_at`, `finished_at` and `duration_ms`, so you can see how parallel workers overlapped.
- The router row and `/status` show how much plan is left from the start, not only after a Claude turn. The last reading per provider is saved across restarts and dropped once its window resets. Codex usage is read from Codex's own session files, and DeepSeek shows its remaining balance (`deepseek $4.20 left`).
- Grok works with an `XAI_API_KEY` instead of `grok login`. Set the key in your shell or in `~/.jev-router.env`: Grok shows as ready, jev-code passes the key to `grok`, and `doctor` offers it as a fix.
- Vim `dd` on the last line of a multi-line message no longer crashes jev-code.
- A repo's `.jev/team.json` pin now respects your own `avoid` and `only` rules for the turn's task. When they exclude the pinned provider, or it cannot do that kind of work, the turn routes automatically and the route line says why. A pinned provider or model that is logged out, cooling down, failed over or unknown is still reported, not swapped.
- jev-code's own git calls (status bar, doctor, completion, repo map, undo, delegate worktrees) no longer run a repo's `core.fsmonitor` program or its hooks, and their diffs skip `textconv` and external diff programs. Filter, merge and credential programs still run, because git-crypt and git-lfs depend on them.
- Privacy rules catch files mentioned with `path:line:col`, `path:10-20` or `path#L4` suffixes, also inside quotes (`` `config/.env`:12 ``).
- Huge day counts in `/sessions clean <days>` and `since:Nd` no longer overflow; they can never delete recent sessions.
- `since:` with a non-ASCII unit (`since:7日`) is refused instead of crashing jev-code.
- Installing an extension refuses to replace a folder of the same name unless jev installed it: it is in the extension list, or its files match its manifest exactly.
- `!command` keeps only the output tail it sends to the model in memory, even for output without line breaks, so long-running noisy commands no longer grow without bound. Long lines reach the model unbroken.
- Cron schedules treat `*/n` in the day fields as unrestricted, as standard cron does (`0 9 */2 * 1` means odd days that are Mondays).

## 1.7.1

A quieter, more premium look for the terminal UI.

- Box borders and rules use a recessive grey, so the frame steps back behind the text.
- The accent (prompt, markers, spinner) is a soft steel blue instead of plain cyan on dark terminals.
- Usage meters use sage, amber and rose instead of raw green, yellow and red.
- Provider colours are unchanged.

## 1.7.0

A live answer, and reasoning you can choose to see.

- The answer is drawn as it is written on Claude, DeepSeek, Gemini and Antigravity, instead of appearing only when the turn is done. The finished answer replaces the preview. Codex and Grok deliver the answer when complete.
- `/thinking on|off` prints the model's whole reasoning, or its one-line trace. The choice is kept for the next session; `JEV_SHOW_THINKING=1` still seeds it.
- The Router panel and `/providers` now show `agy` next to `gemini` when Antigravity is the CLI running it.

## 1.6.0

A `/skill` command, and a clearer terminal UI.

- `/skill` lists your skills in a table, runs one (`/skill db the users table`), shows one before you run it (`/skill show db`) and starts a new one (`/skill new db query the database`). Typing `/skill ` opens a menu of skills with their descriptions and argument hints. Skills in a project's `.claude/skills/` are read too.
- `/help` is easier to scan: descriptions start in a narrower column, so most fit on one line instead of wrapping.
- A failing command's output no longer repeats the line already quoted on its result row.
- Tool calls read at a glance: the tool name is bold with an accent marker, and each result starts with a green ✓, a yellow ✗ (a command that reported failures) or a red ✗ (it failed).
- Between tool calls only the calls show: what the model says moves to the working row, and its reasoning leaves a one-line `✻` trace. `JEV_SHOW_NARRATION=1` brings the narration back.
- The answer has a thin accent bar down its left edge (with colour on).
- jev-code now fills the terminal: the chat on top, the input box under it and the side panel (router, context, branch) as a full-height column on the right. Scroll with the wheel or PageUp/PageDown. Below 100 columns the panel moves under the box. The terminal's own scrollback no longer holds the chat; `JEV_INLINE=1` brings the inline layout back.
- Edits show as a diff: line numbers, with the removed and added lines on red and green rows.

## 1.5.1

A fix for `/compact` on a full session, bounded test concurrency, and an audit log that shows what a long turn did.

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
