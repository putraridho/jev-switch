// Mock scripts, one per state of DESIGN 9.5 plus `mixed`, `warn` and `critical` (every one
// lists all six providers). All times are relative to `FIXTURE_NOW_MS`, Monday 14 Dec 2026
// 12:33 in Asia/Jakarta, the instant the format vectors use; screen tests freeze the page
// clock there so every figure and baseline is reproducible.

import type { HistoryPoint } from "./bindings/HistoryPoint";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { MeterView } from "./bindings/MeterView";
import type { ProviderKey } from "./bindings/ProviderKey";
import type { ReadingView } from "./bindings/ReadingView";
import type { UsageView } from "./bindings/UsageView";
import type { WindowKind } from "./bindings/WindowKind";
import type { ChatSessionState } from "./chat-bindings/ChatSessionState";
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
  | "mixed"
  | "warn"
  | "critical";

/** The instant every fixture is written for: Monday 14 Dec 2026 12:33 (Asia/Jakarta). */
export const FIXTURE_NOW_MS = Date.UTC(2026, 11, 14, 5, 33, 0);

/** A day of 5-hour usage: four windows that each climb and reset, and a three-hour gap. */
function sampleHistory(): HistorySeries {
  const nowS = Math.round(FIXTURE_NOW_MS / 1000);
  const from = nowS - 86_400;
  const window = 18_000;
  const points: HistoryPoint[] = [];
  const resets: number[] = [];
  for (let t = from; t <= nowS; t += 1_800) {
    const into = (t - from) % window;
    if (into === 0 && t > from) resets.push(t);
    // Nothing was seen between 14 and 11 hours ago: a gap, not a line.
    if (t > nowS - 14 * 3_600 && t < nowS - 11 * 3_600) continue;
    // Each window climbs a little differently, so the days do not look copied.
    const climb = 5_500 + (Math.floor((t - from) / window) % 3) * 900;
    const bp = Math.round(300 + (into / window) * climb);
    points.push({ t, v: bp, lo: bp, hi: bp });
  }
  return {
    provider: "claude",
    meter_id: "w:5h",
    from,
    to: nowS,
    unit: "basis_points",
    points,
    gap_s: 3_600,
    resets,
  };
}

function sampleChatState(): ChatSessionState {
  return {
    workspace: "/workspace/project",
    mode: "edit",
    pin: { kind: "auto" },
    routing_enabled: true,
    orchestrate: false,
    jev_key_present: true,
    busy: false,
    active_route: null,
    streaming_text: "",
    streaming_thinking: "",
    streaming_tools: [],
    notes: [],
    pending_permission: null,
    messages: [],
    providers: [
      { provider: "claude", ready: true, detail: "Max 5x", plan: "max_5x" },
      { provider: "codex", ready: true, detail: "Plus", plan: "plus" },
      {
        provider: "deepseek",
        ready: false,
        detail: "no key",
        plan: null,
      },
      { provider: "gemini", ready: true, detail: "Ready", plan: null },
      {
        provider: "grok",
        ready: false,
        detail: "not installed",
        plan: null,
      },
    ],
    models: [
      { provider: "claude", id: "claude-sonnet-5-5" },
      { provider: "claude", id: "claude-opus-5-5" },
      { provider: "claude", id: "claude-haiku-5-5" },
      { provider: "codex", id: "gpt-6.1-sol" },
      { provider: "codex", id: "gpt-6.1-terra" },
      { provider: "gemini", id: "gemini-4-hydrogen" },
      { provider: "deepseek", id: "deepseek-v4-pro" },
      { provider: "grok", id: "grok-4.7" },
    ],
  };
}

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
  history: sampleHistory(),
  chat: sampleChatState(),
  diagnostics: {
    app_version: "0.1.0",
    sources: [
      {
        source: "claude_gateway_log",
        provider: "claude",
        last_ok_at: FIXTURE_NOW_MS - 60_000,
        last_error: null,
        failures: 0,
      },
      {
        source: "codex_app_server",
        provider: "codex",
        last_ok_at: FIXTURE_NOW_MS - 2 * 60_000,
        last_error: "offline",
        failures: 2,
      },
      {
        source: "deepseek_balance",
        provider: "deepseek",
        last_ok_at: null,
        last_error: null,
        failures: 0,
      },
      {
        source: "gemini_none",
        provider: "gemini",
        last_ok_at: null,
        last_error: null,
        failures: 0,
      },
    ],
    history_bytes: 1_240_000,
    history_samples: 8412,
    tray_available: true,
  },
  fail: {},
};

const MINUTE = 60_000;

function windowMeter(
  id: string,
  window: WindowKind,
  usedBp: number,
  resetsInMinutes: number,
  severity: "ok" | "warn" | "critical",
  phase: "current" | "elapsed" = "current",
): MeterView {
  return {
    kind: "window",
    id,
    window,
    used_bp: usedBp,
    resets_at: Math.round((FIXTURE_NOW_MS + resetsInMinutes * MINUTE) / 1000),
    status: severity === "ok" ? "allowed" : "warning",
    severity,
    phase,
  };
}

function reading(
  plan: string | null,
  meters: MeterView[],
  observedAgoMinutes = 1,
): ReadingView {
  return {
    observed_at: FIXTURE_NOW_MS - observedAgoMinutes * MINUTE,
    plan,
    meters,
  };
}

const fiveHour = (
  bp: number,
  severity: "ok" | "warn" | "critical",
  phase?: "current" | "elapsed",
  inMinutes = 134,
) => windowMeter("w:5h", { kind: "five_hour" }, bp, inMinutes, severity, phase);
const sevenDay = (bp: number, severity: "ok" | "warn" | "critical") =>
  windowMeter("w:7d", { kind: "seven_day" }, bp, 3_780, severity);

const claudeKnown: UsageView = {
  state: "known",
  reading: reading("max_5x", [fiveHour(5000, "ok"), sevenDay(3700, "ok")]),
};
const codexKnown: UsageView = {
  state: "known",
  reading: reading("plus", [
    windowMeter("w:5h", { kind: "five_hour" }, 2300, 190, "ok"),
    sevenDay(6150, "ok"),
  ]),
};
const deepseekKnown: UsageView = {
  state: "known",
  reading: reading(null, [
    {
      kind: "balance",
      id: "b:cny",
      currency: "cny",
      total_micros: 110_000_000,
      granted_micros: 10_000_000,
      topped_up_micros: 100_000_000,
      usable: true,
      severity: "ok",
    },
  ]),
};
const unsupported: UsageView = { state: "unsupported" };

type PerProvider = Record<ProviderKey, UsageView>;
function perProvider(
  claude: UsageView,
  codex: UsageView,
  deepseek: UsageView,
): PerProvider {
  return {
    claude,
    codex,
    deepseek,
    gemini: unsupported,
    antigravity: unsupported,
    grok: unsupported,
  };
}
function everyProvider(view: UsageView): PerProvider {
  return {
    claude: view,
    codex: view,
    deepseek: view,
    gemini: view,
    antigravity: view,
    grok: view,
  };
}

const states: Record<FixtureName, PerProvider> = {
  pending: everyProvider({ state: "pending" }),
  known: perProvider(claudeKnown, codexKnown, deepseekKnown),
  stale_quiet: perProvider(
    {
      state: "stale",
      reading: reading(
        "max_5x",
        [fiveHour(5000, "ok"), sevenDay(3700, "ok")],
        125,
      ),
      since: FIXTURE_NOW_MS - 125 * MINUTE,
      cause: { kind: "quiet" },
    },
    codexKnown,
    deepseekKnown,
  ),
  stale_failing: perProvider(
    claudeKnown,
    {
      state: "stale",
      reading: reading("plus", [fiveHour(2300, "ok"), sevenDay(6150, "ok")], 2),
      since: FIXTURE_NOW_MS - 2 * MINUTE,
      cause: { kind: "failing", reason: "offline" },
    },
    deepseekKnown,
  ),
  elapsed: perProvider(
    {
      state: "known",
      reading: reading("max_5x", [
        fiveHour(0, "ok", "elapsed", -10),
        sevenDay(3700, "ok"),
      ]),
    },
    codexKnown,
    deepseekKnown,
  ),
  unavailable: perProvider(
    { state: "unavailable", reason: "offline" },
    { state: "unavailable", reason: "not_logged_in" },
    { state: "unavailable", reason: "unauthorized" },
  ),
  not_configured: perProvider(
    { state: "not_configured", setup: "use_gateway" },
    codexKnown,
    { state: "not_configured", setup: "add_deepseek_key" },
  ),
  first_run: everyProvider({ state: "pending" }),
  mixed: perProvider(
    {
      state: "known",
      reading: reading("max_5x", [
        fiveHour(8600, "warn"),
        sevenDay(3700, "ok"),
      ]),
    },
    codexFailing(),
    { state: "not_configured", setup: "add_deepseek_key" },
  ),
  warn: perProvider(
    {
      state: "known",
      reading: reading("max_5x", [
        fiveHour(8600, "warn"),
        sevenDay(3700, "ok"),
      ]),
    },
    codexKnown,
    deepseekKnown,
  ),
  critical: perProvider(
    {
      state: "known",
      reading: reading("max_5x", [
        fiveHour(9700, "critical"),
        sevenDay(8100, "warn"),
      ]),
    },
    codexKnown,
    deepseekKnown,
  ),
};

function codexFailing(): UsageView {
  return {
    state: "stale",
    reading: reading("plus", [fiveHour(2300, "ok"), sevenDay(6150, "ok")], 2),
    since: FIXTURE_NOW_MS - 2 * MINUTE,
    cause: { kind: "failing", reason: "offline" },
  };
}

function fixture(name: FixtureName): MockScript {
  return {
    ...empty,
    settings: { ...empty.settings, first_run_seen: name !== "first_run" },
    snapshot: {
      ...empty.snapshot,
      generated_at: FIXTURE_NOW_MS,
      providers: empty.snapshot.providers.map((provider) => ({
        ...provider,
        usage: states[name][provider.provider],
      })),
    },
  };
}

const names: readonly FixtureName[] = [
  "pending",
  "known",
  "stale_quiet",
  "stale_failing",
  "elapsed",
  "unavailable",
  "not_configured",
  "first_run",
  "mixed",
  "warn",
  "critical",
];

/** Every fixture name, in a fixed order. */
export const fixtureNames: readonly FixtureName[] = names;

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
  warn: fixture("warn"),
  critical: fixture("critical"),
};
