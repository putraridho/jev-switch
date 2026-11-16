DOCUMENTED, NOT RECORDED: built from the official docs on 2026-11-17; re-record with a logged-in Grok Build.

Sources: xai-org/grok-build `crates/codegen/xai-grok-pager/docs/user-guide/14-headless-mode.md`
(output formats, `streaming-messages-json`, error object), `17-sessions.md`, `22-permissions-and-safety.md`,
and https://docs.x.ai/build/features/sessions.

Format: `grok --single=... --output-format streaming-messages-json` (one JSON object per line, Messages API
`stream-json` shape: `system`/`assistant`/`user`/`result`). `streaming-json` has no line that carries the
whole answer, so a stateless line parser cannot use it.

What is invented, not documented:
- the exact text of the auth and plan error messages (only `{"type":"error","message":"Couldn't start session: ..."}` is documented)
- tool names and input fields inside `tool_use` blocks (the docs show `read_file` with `rawInput.path`)
- ids, token counts and costs

Files:
- `turn1.jsonl`: init, one tool call, answer, result
- `err_auth.jsonl`: `error` line when not logged in
- `err_not_entitled.jsonl`: `result` with `is_error` and `errors[]`, plan without Grok Build
- `empty.jsonl`: success result with an empty answer
