RECORDED 2026-11-17 from `codex app-server` (codex-cli 0.169.0), ChatGPT login, FREE plan.
Transport: newline-delimited JSON-RPC 2.0 over stdio (responses omit the `jsonrpc` field).
Shapes and numeric values are real. Redacted by hand: email, userId, accountId,
chatgptAccountId, installationId, serverName, userAgent tail, codexHome. The JSON-RPC ids
are the ones in `requests.jsonl`.

Timings measured (one process, sequential): initialize 212 ms, account/rateLimits/read
613 ms, account/usage/read 831 ms, account/read 4 ms; process exits with code 0 about
15 ms after stdin is closed; whole session 2.75 s; stderr empty.

Notes for the parser:

- Free plan: `primary.windowDurationMins` = 43200 (30 days), `secondary` = null. Paid plans
  are expected to report a 300-minute primary and a 10080-minute secondary (schema only,
  not recorded). Never assume which window is which: read `windowDurationMins`.
- `usedPercent` is an integer 0..100; `resetsAt` is epoch SECONDS and may be null.
- Unsolicited notifications arrive between responses (`notifications.jsonl`); match
  responses by `id` and ignore everything else.
- `account/read` returns the e-mail address and account ids. The dashboard never calls it
  (the plan comes from `rateLimits.planType`). This fixture exists only so a test can prove
  that feeding it to the parser surfaces nothing.
- The full protocol schema: `codex app-server generate-json-schema --out DIR`
  (v2/GetAccountRateLimitsResponse.json, v2/GetAccountTokenUsageResponse.json).
