DOCUMENTED, NOT RECORDED: built from the official docs on 2026-11-17; re-record with a logged-in CLI.

Sources: https://antigravity.google/docs/cli/headless/ (JSON envelope, `init` / `step_update` /
`result` stream events, status values) and github.com/google-antigravity/antigravity-cli issue 1065
(`SUCCESS` with an empty `response` must be treated as a failure). The contents of `tool_info`,
the `denied_actions` entries and the quota error wording are guesses from third-party write-ups
and are NOT in the official schema. `err_auth.jsonl` is the bare JSON envelope a logged-out
`--output-format json` run is reported to print (the stream-json form is not documented).
