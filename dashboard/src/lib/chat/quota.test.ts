import { describe, expect, it } from "vitest";
import type { MeterView } from "../ipc/bindings/MeterView";
import type { ProviderView } from "../ipc/bindings/ProviderView";
import type { Snapshot } from "../ipc/bindings/Snapshot";
import { planUsage, turnSpend, usageByProvider } from "./quota";

const refresh: ProviderView["refresh"] = {
  in_flight: false,
  last_attempt_at: null,
  next_at: null,
  free_refresh: false,
  live_check: false,
  live_check_error: null,
};

function windowMeter(id: string, used_bp: number): MeterView {
  return {
    kind: "window",
    id,
    window: { kind: id === "7d" ? "seven_day" : "five_hour" },
    used_bp,
    resets_at: null,
    status: "allowed",
    severity: "ok",
    phase: "current",
  };
}

const snapshot: Snapshot = {
  generated_at: 0,
  headline: null,
  providers: [
    {
      provider: "claude",
      enabled: true,
      refresh,
      usage: {
        state: "known",
        reading: {
          observed_at: 0,
          plan: "max_5x",
          meters: [windowMeter("5h", 6200), windowMeter("7d", 3700)],
        },
      },
    },
    {
      provider: "deepseek",
      enabled: true,
      refresh,
      usage: {
        state: "known",
        reading: {
          observed_at: 0,
          plan: null,
          meters: [
            {
              kind: "balance",
              id: "bal",
              currency: "usd",
              total_micros: 1,
              granted_micros: null,
              topped_up_micros: null,
              usable: true,
              severity: "ok",
            },
          ],
        },
      },
    },
    { provider: "grok", enabled: true, refresh, usage: { state: "pending" } },
  ],
};

describe("plan usage", () => {
  it("takes the fullest window", () => {
    const usage = planUsage(snapshot, "claude");
    expect(usage?.meter.id).toBe("5h");
    expect(usage?.plan).toBe("max_5x");
    expect(usage?.stale).toBe(false);
  });

  it("has nothing for balances, pending readings or missing providers", () => {
    expect(planUsage(snapshot, "deepseek")).toBeNull();
    expect(planUsage(snapshot, "grok")).toBeNull();
    expect(planUsage(snapshot, "codex")).toBeNull();
    expect(planUsage(null, "claude")).toBeNull();
  });

  it("remembers each provider's usage", () => {
    expect(usageByProvider(snapshot)).toEqual({ claude: 6200 });
  });

  it("counts only a rise as a turn's spend", () => {
    expect(turnSpend(6200, 6400)).toBe(200);
    expect(turnSpend(6200, 6200)).toBeNull();
    expect(turnSpend(6200, 100)).toBeNull(); // the window reset
    expect(turnSpend(undefined, 6400)).toBeNull();
  });
});
