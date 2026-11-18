# Migrating from 0.2.0 to 1.0.0-rc.1

Nothing has to be changed by hand. Binary names, environment variables and the formats of
sessions, usage logs and settings are unchanged.

What is new on disk, all in the state folder and all created on first use:

- `audit.jsonl`: the audit log. Older versions ignore it; this version skips lines it does not
  understand, so mixed versions are safe.
- `inflight-<pid>.json`: present only while a turn runs (or after a crash, until the next start
  in the same project reports it).

In a project (optional, commit it to share it): `.jev/team.json`.

```json
{
  "max_profile": "standard",
  "routing": [
    { "contains": "migration", "pin": "claude:claude-opus-5-5" },
    { "contains": "typo", "pin": "codex" }
  ]
}
```

- `max_profile` is one of `locked`, `careful`, `standard`, `trusted`. `/mode` and `/profile`
  are lowered to it. Shift-Tab cycling is not limited yet.
- Each `pin` is anything `/model` accepts. The first rule whose text appears in the prompt
  (any case) wins, and only while you have not pinned a model yourself. A bad rule is named by
  its number in `/team`.

`/export` now masks secrets and picks the format from the extension (`.html` or Markdown).
