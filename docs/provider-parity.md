# Provider feature parity audit

Audited 2026-11-19 on branch `provider-feature-parity`, against claude 2.1.328, codex-cli 0.167.0
and agy 1.5.12. Gemini CLI and Grok were not installed, so their rows come from the code only.

jev-code runs each provider's CLI headless (see `src/backend/`), so a feature can reach the user
only if the CLI exposes it headlessly. Each gap is one of three kinds:

- **Pass-through**: the CLI supports it, and jev-code does not pass the flag or input.
- **Display**: the CLI already does it, and jev-code drops the event, so the user never sees it.
- **Out of reach**: the feature exists only in a web app or interactive UI.

## Already working

| Feature | Claude | Codex | Gemini / agy | Grok |
|---|---|---|---|---|
| Session resume | `--resume` | `exec resume` | `--resume` / `session/load` | `--resume` |
| Permission modes | `--permission-mode` | `-s` sandbox | `--approval-mode` / `session/set_mode` | `--permission-mode` + `--allow` |
| Image input | stream-json image blocks | `--image` (new turns only) | paths named in prompt | paths named in prompt |
| Reasoning effort | thinking-budget env var | `model_reasoning_effort` | — | — |
| User's own MCP servers, skills, hooks, plugins | via `--setting-sources` | user config loaded | user config loaded | user config loaded |
| Web search runs | WebSearch tool | yes (checked: on by default) | CLI default | CLI default |

## Gaps

| # | Provider | Feature | Kind | Evidence | Effort |
|---|---|---|---|---|---|
| 1 | Codex | Web search calls hidden | Display | `exec --json` emits `web_search` items; `completed_events` in `codex.rs` handles only `agent_message`, `command_execution`, `file_change`, `reasoning` | S |
| 2 | Codex | MCP tool calls, todo lists, sub-agent items hidden | Display | same parser; item types not yet captured (needs one probe each) | S–M |
| 3 | Codex | Image generation (`image_generation` feature: stable, on) | Display | output item type unknown; the image file needs to be shown or its path printed | M |
| 4 | Codex | Images on resumed turns | Pass-through | `exec resume` gets paths in the prompt, not `--image` (`resume_args`) | S, check `exec resume --help` first |
| 5 | Claude | `--effort` flag | Pass-through | uses a thinking-token env var; CLI now has `--effort <level>` | S |
| 6 | Claude | Claude in Chrome (`--chrome`) | Pass-through | not passed; needs the extension, so opt-in only | S |
| 7 | Claude | AskUserQuestion | Display | disabled on purpose (`--disallowedTools AskUserQuestion`); needs a jev-code prompt UI to answer it | M |
| 8 | Claude | Sub-agent progress | Display | `--forward-subagent-text` not passed; sub-agent text is kept out of the answer | S–M |
| 9 | All | Extra directories (`--add-dir`) | Pass-through | `/add-dir` writes Claude settings only (`permissions::add_directory`); Codex and agy both take `--add-dir` | S |
| 10 | agy | Image input | Pass-through | ACP `session/prompt` sends text only (`gemini/acp.rs`); ACP has image content blocks | M |
| 11 | agy | Effort | Pass-through | agy has `--effort`; ACP backend sets none | S, if ACP accepts it |
| 12 | agy | Plan and command updates | Display | `session/update` kinds `plan`, `available_commands_update` not handled | S |
| 13 | Gemini, Grok | Image input | Pass-through | paths only in prompt text; unverified, CLIs not installed | needs CLI |
| 14 | All | Structured output (`--json-schema`, `--output-schema`) | Pass-through | not exposed; useful for `-p` scripting | M |

## Out of reach

These exist only in a web app or an interactive UI. A headless CLI cannot provide them:

- Claude Artifacts, ChatGPT Sites/Canvas, Gemini Canvas. The nearest substitute is the model
  writing an HTML file and jev-code opening it (a possible `/open` or preview command).
- ChatGPT and Gemini Deep Research, Claude Projects, memory in the web apps.
- Provider-only interactive slash commands. jev-code already has its own `/skill`.

## Suggested order

1. Display gaps 1, 2, 12. The features already run, so this is parsing only, low risk.
2. Pass-throughs 5, 9, 11. Small, each with a unit test on the built command.
3. Gap 3 (Codex image generation) and gap 10 (agy images).
4. Gaps 7 and 8, which need new UI.
