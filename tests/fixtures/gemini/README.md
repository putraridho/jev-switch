DOCUMENTED, NOT RECORDED: built from the official docs on 2026-11-17; re-record with a logged-in CLI.

Sources: https://geminicli.com/docs/cli/headless/ (event names, exit codes) and the
event shapes emitted by the installed `@google/gemini-cli` 0.68.0 bundle
(`type`, `session_id`, `role`/`content`/`delta`, `tool_name`/`tool_id`/`parameters`,
`status`/`stats`/`error`). Things worth knowing when re-recording:

- The `result` event carries NO answer text: the answer is the concatenation of the
  assistant `message` deltas (the parser rebuilds it).
- `result` has an `error` object only when the CLI died with an exception (`err_quota.jsonl`).
  An empty model response ends the stream with an `error` event (severity `error`) and a
  `result` with `status: "error"` and no `error` object (`err_empty.jsonl`).
- The first line of `turn_tools.jsonl` is the plain-text "Loaded cached credentials." line
  the CLI prints, which the parser must ignore.
- `err_auth_stderr.txt` is stderr text only: the CLI exits with code 41 and prints no JSON.
- The quota message wording is representative; Google varies it.
