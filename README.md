# jev-router

A small, reliable LLM router, in Rust. Jev (TypeSafe System One) judges each request;
code maps the judgment to a model and gates the cheap route on confidence. Chat, the
Claude Code gateway and `jev-claude` route between Claude models. `jev-code` is a
coding agent that routes each turn across several providers' coding CLIs (Claude,
Codex, Gemini, DeepSeek, Grok), using only the ones you can really use right now (see
[Code mode](#code-mode-jev-code)). The router fixes the two things that make routers bad:

1. **Memory across switches.** The model has no memory — the *transcript* does.
   One canonical history (`Transcript`) is sent in full to whichever model a turn
   routes to, so continuity survives every switch. (You lose the prompt *cache*
   when you switch models — that's cost, not memory.)
2. **Guessed thresholds.** The cheap-route confidence cutoff is **calibrated from a
   labeled set**, not guessed. `tune` sweeps it and prints the savings/miss curve.

## Design

- **Judge the task, not the model.** Jev returns a `Choice` over `{simple, standard,
  reasoning, unclear}` with described criteria — never "which model".
- **Code owns policy.** Three tiers: cheap (`claude-haiku-5-5`) when `simple` AND
  `confidence ≥ CHEAP_THRESHOLD`; strong (`claude-opus-5-5`) when `reasoning` AND
  `confidence ≥ STRONG_THRESHOLD`; mid (`claude-sonnet-5-5`) for everything else
  (`standard`, `unclear`, or a low-confidence judgment). Defaults 0.50 / 0.85 come
  from `tune` plus a 1000-prompt run: Haiku answers were checked at the cheap boundary,
  and Sonnet matched Opus on all 52 sampled prompts below the strong one (Opus-graded).
- **Hard rules before the gate** (`router::route_turn`): security-sensitive requests
  (auth, passwords, tokens, crypto, sanitizing, CORS…) always go strong, whatever Jev
  says; a short follow-up Jev isn't confident about ("go on", "why?") stays on the
  model that answered the previous turn. `tune` calibrates Jev's judgment only and
  ignores these rules.
- **Verify variant** (recommended, in `router::answer_is_adequate`): run cheap, then
  ask Jev a `Noul` "does this fully answer it?" and escalate if low — the cutoff then
  tracks answer quality, which is far more stable than predicting difficulty up front.

## Run

```bash
export TYPESAFE_API_KEY=...        # Jev (JEV_API_KEY also works)
# Claude runs through Claude Code (`claude -p`) on your subscription — no API key.

# 1) Calibrate the threshold from your labeled cases (no guessing)
cargo run --bin tune                # reads eval/cases.json; prints the sweep + a suggested cutoff

# 2) Chat with the router at that threshold
jev-chat                           # after scripts/install.sh; or: cargo run --bin chat
```

### Claude Code interface (`jev-claude`)

```bash
./scripts/install.sh
cd ~/some-project && jev-claude     # normal Claude Code, with "Jev Code" selected in /model
```

`jev-claude` starts `jev-gateway` (a local Anthropic-format gateway on
127.0.0.1:18780) if needed, then runs `claude` with `ANTHROPIC_BASE_URL` pointing at
it and a custom `/model` entry, **Jev Code** (`ANTHROPIC_CUSTOM_MODEL_OPTION`). Your
claude.ai / Max login passes through unchanged; plain `claude` is unaffected.

- The gateway forwards every request to Anthropic byte-for-byte and streams the
  response back. It changes only the `model` field, and only when it is `jev-code`.
- It decides once per user prompt (Claude Code's `x-claude-code-prompt-id` hint
  header, enabled with `CLAUDE_CODE_GATEWAY_HINT_HEADERS=1`), so all API calls in a
  turn use one model, with code mode's policy: Sonnet by default; Opus for confident
  reasoning, security work (keywords or Jev's `security_review`), or edits to auth
  code; slow downgrades (a tier is held for at most 4 easier turns, even vague ones:
  a cache rebuild pays for itself in about 3 turns against Opus's premium).
  Small background requests (titles, summaries) go to Haiku, with the settings Haiku
  rejects removed (long-context beta, adaptive thinking, effort, system-role messages;
  logged as `adapted`). Background forks of the conversation (they carry the tool
  list) and compaction stay on the turn's model, which also keeps the
  prompt cache warm and avoids Haiku's 200k limit. Subagents that arrive as `jev-code`
  run on Sonnet for the whole run; a model set explicitly on the `Agent` tool is
  forwarded as is.
- Jev judges only what you typed. Local command echoes (`/cockpit`), `!` shell
  output and background-task notices keep the current model without a Jev call. A
  skill command counts as its name and arguments, and pasted text counts as yours.
- A routing judgment takes about 0.5 s. If Jev doesn't answer within
  `JEV_TIMEOUT_MS` (default 3000), the turn goes ahead on its current model.
- Picking Sonnet or Opus in `/model` bypasses routing for that session.
- Logs are in `~/.local/state/jev-router/` (see [Usage log](#usage-log)); gateway
  output is in `gateway.out`. Stop it with `pkill jev-gateway`; `jev-claude` restarts it.
- Claude Code prints an "unrecognized model" note for `jev-code`; it is harmless.
  The `[1m]` suffix tells Claude Code the context window is 1M, like Sonnet and Opus.

#### Orchestrate mode

Off by default; `/orchestrate on` enables it for a session in `jev-claude`. A large prompt that splits into separate pieces of
work ("add X across the API, the UI and the tests") is planned and reviewed by the
strong model and carried out by Sonnet subagents. Everything else routes as above.

- **How Jev decides.** (1) The prompt is orchestrated only if `multi_part` is at the cutoff
  or above **and** normal routing would have sent it to Opus anyway
  (`ORCH_REQUIRE_OPUS`, on by default; `off` restores the multi-part rule alone). In a
  benchmark, five small edits that Sonnet handles alone cost ~8.5x more orchestrated, while
  a hard redesign that routing sent to Opus cost ~21% less orchestrated and finished 35%
  faster (Opus priced at 2x Sonnet, Fable 4x). Skipped prompts are logged with `skipped` and `route_tier`.
  (2) If not, normal routing picks the model. If so, Jev also picks the planner: a turn
  routing escalates as `very_hard` (`FABLE_THRESHOLD`, 0.90) gets Fable, otherwise Opus
  (`ORCH_PLANNER` pins it). (3) Each worker's model is chosen by Jev from the worker's own
  task, as the planner spawns it (`POST /jev/worker`), from Jev's own judgments (security
  review, reasoning, edits) and not the security keyword lists: a worker's brief is long and
  full of incidental words, and on a concurrency redesign the keyword rule tagged every
  worker security-sensitive. Fable is never a worker. Opus can be
  one only when Fable is the planner, for a task Jev judges security-sensitive or hard;
  under an Opus planner every worker is Sonnet (a security task included, which gets the
  planner's closest review). Haiku is for a read-only `simple` task, and only if
  `WORKER_CHEAP_P` is set (off by default, uncalibrated). The planner's own `model` choice stands only if
  Jev can't be reached. (4) The planner reviews the results itself: reads the diffs, runs
  the checks, and sends short steps back out before reporting.
- **Trigger.** The `jev-orchestrate` mod (`mods/jev-orchestrate`, loaded by
  `jev-claude` with `--plugin-dir`) sends each typed prompt to the gateway's
  `POST /jev/orchestrate`, which asks Jev a `multi_part` Noul. At `ORCH_TRIGGER_P`
  (0.70) or above, the mod attaches a brief the user doesn't see: plan, delegate every
  edit to the Agent tool, one subagent per step and independent steps in parallel, each
  with `model: "sonnet"` (`"opus"` for security steps), review,
  and make no edits itself. The gateway sees the brief's `<jev-orchestrate>` marker and
  runs the turn's main thread on `ORCH_PLANNER` (default `claude-opus-5-5`;
  `claude-fable-5-2` also works). Subagents name their own model, so they pass through.
- **Cache-safe.** The brief is part of Claude Code's own transcript, so later requests
  carry it unchanged; the gateway never edits a request to orchestrate.
- **Toggle.** `/orchestrate` toggles it for the current session; `/orchestrate on` and
  `/orchestrate off` set it. New sessions start off. The status line shows
  `orchestrate: on`, `orchestrate: off`, or `orchestrating` during a triggered turn.
  `JEV_ORCH=off` in the gateway's environment turns it off everywhere.
- **Failure is a normal prompt.** If the gateway is down, slow (5 s) or Jev fails, the
  prompt goes through unchanged. Other models, slash commands and messages delivered
  into a running turn are never checked.
- **Cutoff.** `orch_tune` scored 1,551 distinct logged prompts: 12 (0.8%) were at 0.70
  or above, all but one real multi-part work; 0.60-0.70 mixes in follow-ups and single
  asks. The gateway log keeps only 200 characters of a prompt, so long prompts were
  scored on their start.
- **Cost.** Off by default, and gated on normal routing choosing Opus, because on work
  Sonnet handles alone it only adds an expensive planner. Two benchmarks, one run per arm,
  all results passing `cargo test`, priced at Claude Code's own estimate (Opus 2x Sonnet,
  Fable 4x, measured with `claude -p`; an estimate, not a bill):
  - Five small edits across five files: orchestrated ~2.9x a Sonnet-only run in the first
    benchmark and ~8.5x in the second (the Sonnet baseline itself varied 2x), and not faster.
  - A hard gateway-concurrency redesign that normal routing sent to Opus: orchestrated cost
    ~21% less and finished 35% faster (Opus planner, Sonnet workers).
  The Fable-planner and Opus-worker path is unmeasured. `scripts/orch_report.py` prices real
  tasks. Turn it on per session with `/orchestrate on`; `JEV_ORCH=off` in the gateway's
  environment disables the check everywhere.
- **Logs.** `orch.jsonl` (`JEV_ORCH_LOG`): one line per check (`multi_part_p`,
  `trigger`, `cutoff`, `planner`, `hard_p`, prompt start) and one per worker
  (`kind: "worker"`, `model`, `planner`, `prompt_id`, `tool_use_id`, `reason`, `signals`).
  Every `usage.jsonl` row has `orch_planner`, set while the session's turn is orchestrated
  (workers' rows included). `scripts/orch_report.py --exclude PREFIX` skips test sessions. Each turn's `decision.orchestrate`
  in `turns.jsonl`/`usage.jsonl` carries the check, triggered or not, and an
  orchestrated turn has `reason: "orchestrate"`. An orchestrated turn with no `Agent`
  call in `tools` is a planner that didn't delegate; an untriggered turn just under the
  cutoff with many tool calls is a likely miss. Both feed the next retune.

#### Usage log

The gateway writes three JSON-lines files (mode 0600) for later analysis. They cover
every `/v1/messages` request, including ones you send to Sonnet or Opus directly
(`routed: false`), which gives you a baseline to compare routing against.

- `gateway.jsonl`: one line per routing decision: `ts`, `session`, `prompt_id`,
  `model`, `reason`, `signals`, and the first 200 characters of the prompt. It is the
  only file with any prompt text.
- `usage.jsonl`: one line per request.
  - **Who/where:** `ts_ms`, `session`, `prompt_id`, `class` (`main`, `auxiliary`,
    subagent…), `cwd`, `project`, `client`.
  - **Routing:** `requested_model`, `model`, `routed`, `adapted`, `new_turn`, `decision` (`tier`,
    `reason`, `confident`, Jev's `signals`: `choice`, `confidence`, `edit_p`, `sec_p`,
    `security`; the same for every request of a turn).
  - **Request shape:** `params` (`max_tokens`, `thinking`, `context_management`, …),
    `beta`, `stream`, `messages`, `tools`, `req_bytes`.
  - **Result:** `status`, `request_id`, `message_id`, `response_model`, `stop_reason`,
    `error`, `complete` (false if the client aborted mid-stream), `content_encoding`
    (set only if a response was compressed and so couldn't be read).
  - **Cost:** `usage`, exactly as Anthropic returns it (`input_tokens`,
    `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`,
    `output_tokens_details.thinking_tokens`, …).
  - **Quota:** `ratelimit`: the `anthropic-ratelimit-*` response headers (on a Max
    login, 5-hour and 7-day utilization and reset times) and `retry-after`.
  - **Behavior:** `blocks` (counts by type: text, thinking, tool_use…),
    `tools_called` (tool names only, no arguments), `ttfb_ms`, `latency_ms`,
    `resp_bytes`.
- `turns.jsonl`: one line per user turn, summing its requests (`requests`, `by_class`,
  `models`, `tokens`, `tools`, `errors`, `wall_ms`, `ended`: `end_turn`, `max_tokens`,
  `aborted`, `error` or `superseded`) plus the **outcome signals**, in `followup`,
  describing how the user reacted to the *previous* turn:
  - `gap_ms`: time from the previous answer to this prompt.
  - `similarity`: word overlap with the previous prompt (near 1 = asked again).
  - `model_switched`, `prev_requested_model`: you changed `/model` between turns.
  - `correction_p` (opt-in, `JEV_CORRECTION_CHECK=1`): Jev's probability that the
    prompt reacts to the last answer as wrong or incomplete (a background call; never
    used for routing). Off by default: on 276 logged turns it did not track aborts,
    gaps or model switches, so it needs real labels before it is worth a call per turn.

  To judge routing, read turn *n+1*'s `followup` as a verdict on turn *n*. An `aborted`
  turn means you interrupted it. Turns with no `prompt_id` header are not rolled up.

The tap reads the usage events as the response streams past and never changes what the
client receives. It does remove `Accept-Encoding` from logged requests so responses
arrive uncompressed. Join files on `(session, prompt_id)`. Example:

```bash
jq -s 'group_by(.model)[] | {model: .[0].model, requests: length,
  out: (map(.usage.output_tokens // 0) | add)}' ~/.local/state/jev-router/usage.jsonl
```

`prefix` says how a request's cache prefix compares with its closest predecessor:
`intact` (the predecessor is wholly contained), `tail_only` (only its last message
changed: cheap, ~6k tokens rebuilt), or a real break with `first_diff` naming the
block, plus `system_changed` / `tools_changed`. Count only non-intact, non-`tail_only`
requests as cache breaks.

Paths: `JEV_USAGE_LOG`, `JEV_TURNS_LOG`, `JEV_GATEWAY_LOG`. A file over
`JEV_LOG_MAX_MB` (default 50, 0 = never) is moved to `<name>.1`, replacing the previous
one.

Failed requests print a `[warn]` line to `gateway.out`, with the full cause chain (DNS, refused
connection, TLS). A connection that can't be established is retried twice (250 ms, then 1 s);
the usage row records `retries`. Nothing is retried once a request has reached Anthropic. If Haiku rejects a background
request (400/413/422) or rate-limits it (429), the gateway retries it once on Sonnet as the client sent it and
records `fallback_from` (model, status, error) on the usage row.

### Code mode (`jev-code`)

```bash
./scripts/install.sh          # builds release, installs ~/.local/bin/jev-code and jev-chat
cd ~/some-project && jev-code # or: jev-code /path/to/project; jev-code --help
```

The Jev key is read from `TYPESAFE_API_KEY` / `JEV_API_KEY`, or from
`~/.jev-router.env` (`JEV_API_KEY=...`), so it works from any directory. Re-run
`scripts/install.sh` after changing the code.

A routed coding agent. Every turn runs one provider's coding CLI in your project with
tools on. Jev says what kind of task the turn is (security, hard debugging, review,
feature, frontend, docs, explain, current info, simple edit, unclear); a catalog maps
that to an ordered list of models; the router takes the first one you can use.

**Providers and what each needs**

| provider | needs | billing |
|---|---|---|
| claude | `claude` installed and logged in | your plan; Fable only with `JEV_ALLOW_CREDITS=1` |
| codex | `codex` installed and logged in (ChatGPT plan) | your plan |
| gemini | `gemini` with auth configured, else `agy` (Antigravity) | your plan |
| grok | Grok Build installed | your plan |
| deepseek | `claude` installed and `DEEPSEEK_API_KEY` (env or `~/.jev-router.env`) | per token; setting the key is the opt-in |

**Never routes to a provider you cannot use.** At startup each provider is probed
(installed? logged in? key present?) and only ready ones print. A probe that cannot
tell counts as not ready. `/providers` shows everything with the reason, plus
cooldowns, refused models and billing allowances; `/providers refresh` probes again.
If nothing usable fits a turn, it says so in plain words and starts nothing; a pinned
provider that is unusable is reported, never swapped for another.

**Failover and cooldown.** A usage limit puts that provider on a cooldown (until its
reset time when the CLI gives one, else an hour; saved across runs) and the same
prompt goes to the next model. A model your plan does not include is dropped for the
run. A login failure cools the provider and tells you which tool to log in to again.
An empty answer or a stall skips that model for this turn only. Any other error stops
the turn, since it would likely recur everywhere. At most 6 models are tried per
turn, and a failed attempt's output is never shown as an answer.

**Switching providers.** Each CLI keeps its own session, so when the turn goes to a
provider that missed earlier turns, a short catch-up (your prompts, shrunk replies,
files touched, commands run, `git status`) is put in front of the prompt. Switching
costs a cold session, so the router changes provider only for a confident new task,
likely correction, or security work; within a provider an upgrade is immediate and a
downgrade is held for a few turns (the prompt cache).

**Hard security rule.** Security work (strong keywords, a high `security_review`, or
auth/session edits) overrides Jev's task label and never goes to DeepSeek or Grok.

**Commands.** `/model auto` | `<provider>` | `<provider>:<model>` | `sonnet|opus|haiku`
(pins are never silently overridden), `/provider auto|off`, `/providers [refresh]`,
`/new`, `/quit`.

**Environment.** `JEV_PROVIDERS=off` starts with routing off; `JEV_ALLOW_CREDITS=1`
allows credit-billed models; `JEV_CHAINS` is a JSON file replacing the model order per
task type; `DEEPSEEK_API_KEY`; `JEV_TIMEOUT_MS` (Jev wait, default 3000; past it the
turn is routed without Jev's judgment); `CODE_IDLE_TIMEOUT_SECS` (default 900);
`JEV_CODE_LOG` (turn log, one row per attempt with the decision reason, Jev's signals,
attempt number and failover notes). `jev-code --help` lists the rest.

**Permissions.** Edits are auto-accepted on Claude (`CODE_PERMISSION_MODE=acceptEdits`);
its commands need your Claude Code allowlist or `CODE_ALLOWED_TOOLS="Bash(cargo test) Bash(git status)"`.
Codex runs in a workspace-write sandbox (`CODEX_SANDBOX`, never full access). Denied
actions are reported, never silently skipped. Use it inside a git repo.

**How far to trust each adapter.** Verified against real recorded CLI output: claude
and codex (`tests/fixtures/{claude,codex}`). Built from the vendors' documentation only,
never run against a logged-in CLI, so expect corrections on first real use: gemini and
agy (`tests/fixtures/{gemini,agy}`) and grok (`tests/fixtures/grok`). DeepSeek runs the
`claude` CLI with `ANTHROPIC_BASE_URL` pointed at DeepSeek: the key check is real, but
that this really bypasses your Claude login and talks to DeepSeek is unverified. Model
names, tiers and chains in the catalog come from the vendors' pages as of 2026-11-17;
the task-to-model chains are defaults, not calibrated on your work.

**Orchestration across providers** (`/orchestrate on|off`, or `JEV_ORCHESTRATE=on`; off by
default). A request that Jev scores multi-part (`multi_part` at `ORCH_TRIGGER_P`, 0.70) AND
that normal routing would send to a Top-tier model runs on a PLANNER, which gets a
`delegate` tool (an MCP server, `delegate_mcp`). The planner hands independent pieces to
cheaper WORKERS, each in its own git worktree (`<state dir>/worktrees/<id>`, detached at
`HEAD`, so the planner's uncommitted edits are not visible to workers), reviews the diffs it
gets back, and `merge`s or `discard`s them. The `delegate` result lists EVERY changed file with
its +/- counts and shows whole-file diffs up to 20,000 characters; `show_diff` (worker_id,
optional `file`, `offset`) pages the rest, and `merge` (optional `files`) refuses any file the
planner has not seen in full. A merge is a plain `git apply` onto the working tree (so the
planner's unstaged edits to other hunks of the same file survive), falling back to `--3way`; a
failure changes nothing and says whether it is a conflict or an overlapping uncommitted edit.
Unmerged workers are saved as `<state dir>/workers/<id>.json` and reloaded after a restart;
checkouts older than 24 hours and unlisted directories under `worktrees/` are removed at
startup and `git worktree prune` runs. **Jev makes every model decision:** the tool has no model parameter and anything a
planner says about models is ignored. Code only filters to usable models and applies hard
rules: workers are never Top tier and never Fable/Astra/Carbon, security briefs never go
to DeepSeek or Grok, code-writing work never drops below Everyday (a brief the planner calls
read-only may use Light only if Jev agrees it does not edit), workers are spread across
providers by plan usage and by workers already running, and a worker that hits a limit,
a login or an entitlement failure is retried once on the next pick. Planners can be claude,
deepseek (through claude) and codex: gemini and grok cannot take the tool yet, so they are
never chosen as planner; with no planner available the turn runs normally and the
`[route]` line says why. Every worker attempt is a row in `<state dir>/orchestration.jsonl`
(planner, worker, Jev's signals, ok, wall time, files changed, usage); `scripts/orch_report.py`
does not read it yet. Codex attaching the tool through `-c mcp_servers...` overrides was
checked against `codex exec --help` only, not run.

**Routing off** (`/provider off` or `JEV_PROVIDERS=off`) is the earlier Claude-only
mode: one attempt, no availability check, no failover, two tiers and no Haiku:

- Sonnet by default; Opus for confident reasoning (`CODE_STRONG_THRESHOLD=0.85`),
  strong security terms (vulnerabilities, secrets, crypto, audits), security
  questions without those terms, and edits to auth/session code.
- Security questions without keywords ("can a viewer see who the org shares files
  with?", "who can call this?") are caught by a third Jev judgment,
  `security_review`, at `CODE_SECURITY_P` (0.85).
- Both cutoffs come from lunagraph: 2,296 prompts over the whole monorepo, routed
  with every Jev signal recorded (`ROUTE_SIGNALS=1`), 408 Sonnet answers graded
  against the code by Opus, and Opus reruns of every Sonnet miss, graded the same
  way. Sonnet was correct on 383 (94%). Replaying candidate policies on the
  recorded signals:

  | policy | turns on Opus | Sonnet misses Opus fixes, left on Sonnet |
  |---|---|---|
  | reasoning 0.75, security 0.90 (before) | 31% | 7 |
  | reasoning 0.85, security 0.85 (now) | 28% | 5 |
  | reasoning 0.90, security 0.85 | 26% | 6 |
  | security only, 0.85 (no reasoning rule) | 15% | 7 |

  Sonnet's answers at reasoning confidence 0.75–0.85 were 31 correct and 2 partial
  that Opus also got partly wrong. The keyword-less security questions it missed and
  Opus fixed scored 0.86–0.97. The 5 left are misses no signal predicts: a refactor
  and a small edit Jev called simple, two security-sounding questions that weren't
  (a theme-token lookup, md5 for content hashes), and one security question Jev scored
  0.60. Strong keywords still go to Opus unconditionally: gating them on
  `security_review` would save under 2% and they also cover Jev being down.
- No Haiku. Tested on five repos (2,500 questions, 120 answers verified against the
  code): Haiku's code explanations held up ~85% at every Jev confidence, so no
  cutoff separated its misses, and prompt caches are per model, so its cache-cold
  turns saved little. Sonnet held up on 60 of 68 graded answers.
- Upgrades are immediate; stepping back down to Sonnet needs `DOWNGRADE_AFTER` (2)
  confident easier turns, because each switch rebuilds the context cache. Short
  replies Jev calls `unclear` count as easier when `edit p` < 0.2, and an uncertain
  turn holds the streak instead of resetting it; only a turn that needs the current
  tier resets it.

`eval/cases.json` is a list of `{ request, cheap_was_fine }` — the ground truth is
whether the **cheap** model actually handled each request well. 20–30 real examples
beat any guess. Re-run `tune` when prompts, models, or request mix change.

## Files

- `src/anthropic.rs` — Claude client via Claude Code (`claude -p`); sends the full history each call.
- `src/typesafe.rs` — Jev client (`choice`, `noul`).
- `src/router.rs` — the routing judgment + confidence gate + verify-Noul.
- `src/transcript.rs` — the one canonical conversation history.
- `src/bin/chat.rs` — interactive router REPL.
- `src/bin/tune.rs` — threshold calibration (the sweep).
- `src/bin/gateway.rs` — local gateway that adds the Jev Code model to Claude Code.
- `scripts/jev-claude` — launcher: starts the gateway and opens Claude Code on Jev Code.
- `src/bin/code.rs` — thin REPL for the routed coding agent; `src/turn.rs` runs one turn
  (Jev, choose, hand off, run, fail over), `src/routing.rs` picks the model, `src/catalog.rs`
  lists the models and chains, `src/providers.rs` and `src/availability.rs` know who is
  usable, `src/handoff.rs` writes the catch-up, `src/backend/` has one adapter per CLI.
- `src/delegate.rs`, `src/orchestrate.rs`, `src/bin/delegate_mcp.rs` — cross-provider orchestration:
  worker choice and worktrees, planner choice and tool attachment, the MCP server.
- `src/bin/orch_tune.rs` — scores logged prompts with `multi_part` to pick `ORCH_TRIGGER_P`.
- `mods/jev-orchestrate/` — Claude Code mod for orchestrate mode (trigger, brief, `/orchestrate`).
- `src/bin/route.rs` — routing-only dry run over a prompt file (Jev only, no Claude calls);
  `ROUTE_MODE=code ROUTE_SIGNALS=1` also records every Jev judgment per prompt for
  replaying policies offline.

## Notes

- Keys stay in env / server-side.
- Set `CLAUDE_BIN` if `claude` isn't on your PATH.
- Thresholds aren't permanent; they drift with your inputs — recalibrate periodically.
