// Mock scripts, one per state of DESIGN 9.5 plus `mixed`. The names are final; T07 fills
// each script (every one lists all six providers). This stub gives every name the same
// empty script.

import type { MockScript } from "./mock";

/** The name of a fixture script. */
export type FixtureName =
  | "pending"
  | "known"
  | "stale_quiet"
  | "stale_failing"
  | "elapsed"
  | "unavailable"
  | "not_configured"
  | "first_run"
  | "mixed";

const empty: MockScript = {
  snapshot: { generated_at: 0, providers: [], headline: null },
  settings: {
    version: 1,
    warn_bp: 8000,
    critical_bp: 9500,
    notifications: true,
    notify_warn: true,
    notify_critical: true,
    theme: "system",
    hour_cycle: "auto",
    zoom_pct: 100,
    close_to_tray: true,
    autostart: true,
    shortcuts: true,
    claude_live_check: false,
    first_run_seen: false,
    hidden_providers: [],
  },
  history: {
    provider: "claude",
    meter_id: "w:5h",
    from: 0,
    to: 0,
    unit: "basis_points",
    points: [],
    gap_s: 0,
    resets: [],
  },
  diagnostics: {
    app_version: "0.1.0",
    sources: [],
    history_bytes: 0,
    history_samples: 0,
    tray_available: false,
  },
  fail: {},
};

/** A whole script per name: `first_run` differs in `settings`, not in the snapshot. */
export const fixtures: Record<FixtureName, MockScript> = {
  pending: empty,
  known: empty,
  stale_quiet: empty,
  stale_failing: empty,
  elapsed: empty,
  unavailable: empty,
  not_configured: empty,
  first_run: empty,
  mixed: empty,
};
