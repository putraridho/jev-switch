# Editor bridge protocol (version 1)

The contract an editor plugin speaks to a running `jev-code`: one JSON object per line. It is
defined in `src/bridge.rs` (typed, with tests). The transport (pipe or socket) and the plugins
themselves are not built yet.

Editor to jev-code (`"type"` selects the message):

```
{"type":"hello","client":"vscode","protocol":1}
{"type":"focus","path":"src/a.rs","line":12}
{"type":"selection","path":"src/a.rs","start_line":3,"end_line":9,"text":"..."}
{"type":"diagnostics","path":"src/a.rs","items":[{"line":3,"severity":"warning","message":"unused"}]}
{"type":"prompt","text":"explain this"}
{"type":"interrupt"}
```

jev-code to the editor:

```
{"type":"ready","version":"1.0.0","protocol":1}
{"type":"ack"}
{"type":"file_changed","path":"src/a.rs"}
{"type":"approval_needed","tool":"Bash","summary":"cargo test"}
{"type":"turn_finished","ok":true}
{"type":"error","message":"..."}
```

A `hello` with another protocol number gets an `error` saying which side is older.
