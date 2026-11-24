SYNTHETIC. Nothing here was copied from a real `usage.jsonl`; no real log was opened to make it.
Rows have the shape of DESIGN 3.2 (compact JSON, `ts_ms` at request start, every `ratelimit`
value a string) with invented values. `ts_ms` values are deliberately not in order.

- `sample.jsonl`: 12 rows. Rows 2, 8 and 12 have an empty `ratelimit`. Row 3 carries the
  sporadic `7d_oi` window. Row 4 is older than row 3; the last sample row (11) is older than
  row 10, which is the newest sample (`ts_ms` 1794890055000: 5h 2 %, 7d 40 %).
- `canary.jsonl`: 5 rows, each still a valid sample (5h 50 %, 7d 37 %). The canary
  `JEVCANARY_9f3c1b7e_DO_NOT_SURFACE` sits in every non-allow-listed key, as extra keys, at
  depth, inside allow-listed keys' values and as a window name; the label-shaped canary
  `jevcanary_9f3c1b7e` sits in every position that is NOT a label position of DESIGN 8.3.
  Nothing parsed from this file may contain either string or `canary-email`.
- `label_canary_as_window.jsonl`: the converse, by design: the label-shaped string as a
  window name (a label position) does surface, as `Named(jevcanary_9f3c1b7e)`.
- `rotation/usage.jsonl.1` + `rotation/usage.jsonl`: one log split by a rotation. The first
  row of the new file is older than the last row of the old one; the newest sample is
  `ts_ms` 1794890180000 (5h 36 %, 7d 25 %), which is not the last line.
