DOCUMENTED, NOT RECORDED: built from the official docs on 2026-11-17; re-record with a logged-in CLI.

Sources: https://antigravity.google/docs/cli/headless/ (JSON envelope, `init` / `step_update` /
`result` stream events, status values) and github.com/google-antigravity/antigravity-cli issue 1065
(`SUCCESS` with an empty `response` must be treated as a failure). The contents of `tool_info`,
the `denied_actions` entries and the quota error wording are guesses from third-party write-ups
and are NOT in the official schema. `err_auth.jsonl` is the bare JSON envelope a logged-out
`--output-format json` run is reported to print (the stream-json form is not documented).

The production adapter now uses `agy acp`, not headless print mode. These fixtures
remain as regression evidence for the old soft-denial/empty-response behavior.
ACP setup, mode selection, permission round trips, resumed-history isolation and
answer assembly are tested in `src/backend/gemini/acp.rs`, including a simulated
CLI driven through the real process runner. The installed `agy` 1.5.10 completed
ACP initialization; a real session could not start under the development sandbox
(`listen tcp 127.0.0.1:0: socket: operation not permitted`). A live model/tool
turn is still unverified. Protocol references:

- https://antigravity.google/docs/cli/acp/
- https://agentclientprotocol.com/protocol/v1/session-setup
- https://agentclientprotocol.com/protocol/v1/tool-calls
