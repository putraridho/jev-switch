// Mock scripts, one per state of DESIGN 9.5 plus `mixed`. The names are final; T07 fills
// each script (every one lists all six providers). This stub gives every name the same
// empty script.

import type { ProviderKey } from "./bindings/ProviderKey";
import type { UsageView } from "./bindings/UsageView";
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
  snapshot: {
    generated_at: 1700000000000,
    providers: (
      ["claude", "codex", "deepseek", "gemini", "antigravity", "grok"] as const
    ).map((provider) => ({
      provider,
      enabled: true,
      usage: { state: "pending" },
      refresh: {
        in_flight: false,
        last_attempt_at: null,
        next_at: null,
        free_refresh: true,
        live_check: false,
        live_check_error: null,
      },
    })),
    headline: null,
  },
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

const reading = { observed_at: 1700000000000, plan: "Example", meters: [] };
const states: Record<FixtureName, UsageView> = {
  pending: { state: "pending" },
  known: { state: "known", reading },
  stale_quiet: {
    state: "stale",
    reading,
    since: 1700000000000,
    cause: { kind: "quiet" },
  },
  stale_failing: {
    state: "stale",
    reading,
    since: 1700000000000,
    cause: { kind: "failing", reason: "offline" },
  },
  elapsed: { state: "known", reading },
  unavailable: { state: "unavailable", reason: "offline" },
  not_configured: { state: "not_configured", setup: "use_gateway" },
  first_run: { state: "pending" },
  mixed: { state: "pending" },
};
const mixed: Record<ProviderKey, UsageView> = {
  claude: states.known,
  codex: states.stale_quiet,
  deepseek: states.unavailable,
  gemini: states.pending,
  antigravity: states.not_configured,
  grok: states.stale_failing,
};
function fixture(name: FixtureName): MockScript {
  return {
    ...empty,
    settings: { ...empty.settings, first_run_seen: name !== "first_run" },
    snapshot: {
      ...empty.snapshot,
      providers: empty.snapshot.providers.map((provider) => ({
        ...provider,
        usage: name === "mixed" ? mixed[provider.provider] : states[name],
      })),
    },
  };
}
export const fixtures: Record<FixtureName, MockScript> = {
  pending: fixture("pending"),
  known: fixture("known"),
  stale_quiet: fixture("stale_quiet"),
  stale_failing: fixture("stale_failing"),
  elapsed: fixture("elapsed"),
  unavailable: fixture("unavailable"),
  not_configured: fixture("not_configured"),
  first_run: fixture("first_run"),
  mixed: fixture("mixed"),
};
