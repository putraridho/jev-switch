import { describe, expect, it } from "vitest";
import {
  firstRunLine,
  levelChange,
  planName,
  providerName,
  providerOrder,
  retryable,
  setupCopy,
  sourceLine,
  staleNote,
  stateChange,
  statusName,
  unavailableCopy,
  unsupportedCopy,
  unsupportedLine,
} from "./copy";
import type { MeterView } from "./ipc/bindings/MeterView";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import type { Unavailable } from "./ipc/bindings/Unavailable";
import type { UsageView } from "./ipc/bindings/UsageView";

const reasons: readonly Unavailable[] = [
  "not_installed",
  "not_logged_in",
  "offline",
  "timeout",
  "rate_limited",
  "unauthorized",
  "server_error",
  "protocol",
  "io",
  "internal",
];

describe("copy", () => {
  it("names every provider and orders them as ProviderKey::ALL", () => {
    expect(providerOrder.map((key) => providerName[key])).toEqual([
      "Claude",
      "Codex",
      "DeepSeek",
      "Gemini",
      "Antigravity",
      "Grok",
    ]);
  });

  it("gives every failure one plain sentence with no code and no blame", () => {
    for (const reason of reasons) {
      for (const provider of providerOrder) {
        const text = unavailableCopy(reason, provider).text;
        expect(text.length).toBeGreaterThan(10);
        expect(text).not.toMatch(/error|exception|!|something went wrong/i);
        expect(text.endsWith(".")).toBe(true);
      }
    }
  });

  it("offers the engine's login commands where there are some", () => {
    expect(unavailableCopy("not_logged_in", "codex").command).toBe(
      "codex login",
    );
    expect(unavailableCopy("not_logged_in", "claude").command).toBe(
      "claude auth login",
    );
    expect(unavailableCopy("unauthorized", "codex").command).toBe(
      "codex login",
    );
    expect(
      unavailableCopy("not_logged_in", "deepseek").command,
    ).toBeUndefined();
    expect(unavailableCopy("unauthorized", "deepseek").command).toBeUndefined();
    expect(unavailableCopy("offline", "codex").command).toBeUndefined();
  });

  it("says what to do first", () => {
    expect(setupCopy("use_gateway", "claude")).toEqual({
      text: "No Claude usage seen yet. Run jev-claude once and it appears here.",
      command: "jev-claude",
    });
    expect(setupCopy("add_deepseek_key", "deepseek").text).toContain("key");
  });

  it("is honest about providers with no source", () => {
    expect(unsupportedCopy("grok")).toBe("Grok has no usage source yet.");
    expect(unsupportedLine(["gemini", "antigravity", "grok"])).toBe(
      "Gemini, Antigravity and Grok have no usage source yet.",
    );
    expect(unsupportedLine(["grok"])).toBe("Grok has no usage source yet.");
    expect(unsupportedLine([])).toBe("");
  });

  it("turns plan labels into words", () => {
    expect(planName("max_5x")).toBe("Max 5x");
    expect(planName("free")).toBe("Free");
    expect(planName("")).toBe("");
  });

  it("words every status", () => {
    expect(statusName.rejected).toBe("Limit reached");
  });
});

const window = (
  severity: "ok" | "warn" | "critical",
  resets: number | null,
): MeterView => ({
  kind: "window",
  id: "w:5h",
  window: { kind: "five_hour" },
  used_bp: 8260,
  resets_at: resets,
  status: "warning",
  severity,
  phase: "current",
});

describe("levelChange", () => {
  const now = 1797226380000;
  it("reads like the notification", () => {
    expect(levelChange("claude", window("warn", 1797231600), now, "h24")).toBe(
      "Claude 5-hour at 82%. Resets today 14:00.",
    );
  });
  it("leaves the reset out when it is unknown", () => {
    expect(levelChange("claude", window("critical", null), now, "h24")).toBe(
      "Claude 5-hour at 82%.",
    );
  });
  it("says when a level is back to normal", () => {
    expect(levelChange("codex", window("ok", null), now, "h24")).toBe(
      "Codex 5-hour is back to normal at 82%.",
    );
  });
  it("describes a balance", () => {
    const balance: MeterView = {
      kind: "balance",
      id: "b:cny",
      currency: "cny",
      total_micros: 1,
      granted_micros: null,
      topped_up_micros: null,
      usable: false,
      severity: "critical",
    };
    expect(levelChange("deepseek", balance, now, "h24")).toBe(
      "DeepSeek balance is not usable.",
    );
    expect(
      levelChange("deepseek", { ...balance, usable: true }, now, "h24"),
    ).toBe("DeepSeek balance is available.");
  });
});

describe("stateChange", () => {
  const reading = { observed_at: 0, plan: null, meters: [] };
  const known: UsageView = { state: "known", reading };
  const pending: UsageView = { state: "pending" };
  const quiet: UsageView = {
    state: "stale",
    reading,
    since: 0,
    cause: { kind: "quiet" },
  };
  const failing: UsageView = {
    state: "stale",
    reading,
    since: 0,
    cause: { kind: "failing", reason: "offline" },
  };
  const change = (
    a: UsageView | undefined,
    b: UsageView,
    p: ProviderKey = "codex",
  ) => stateChange(p, a, b);

  it("is silent for the first snapshot and for no change", () => {
    expect(change(undefined, known)).toBeNull();
    expect(change(known, known)).toBeNull();
    expect(change(quiet, quiet)).toBeNull();
    expect(change(pending, { state: "unsupported" })).toBeNull();
    expect(change(known, pending)).toBeNull();
  });

  it("is silent when the first answer arrives", () => {
    expect(change(pending, known)).toBeNull();
  });

  it("announces a provider that can't be reached", () => {
    expect(change(known, failing)).toBe(
      "Can't reach Codex. Showing the last reading.",
    );
    expect(change(known, quiet)).toBe(
      "Codex is quiet. Showing the last reading.",
    );
    expect(change(quiet, failing)).toBe(
      "Can't reach Codex. Showing the last reading.",
    );
    expect(change(failing, quiet)).toBe(
      "Codex is quiet. Showing the last reading.",
    );
    expect(
      change(known, { state: "unavailable", reason: "offline" }),
    ).toContain("Can't reach Codex");
  });

  it("announces a recovery and a setup need", () => {
    expect(change(failing, known)).toBe("Codex is back.");
    expect(
      change(
        known,
        { state: "not_configured", setup: "use_gateway" },
        "claude",
      ),
    ).toContain("jev-claude");
  });
});

describe("staleNote", () => {
  const reading = { observed_at: 1797219000000, plan: null, meters: [] };
  const now = 1797226380000;
  it("is null unless the reading is stale", () => {
    expect(staleNote("claude", { state: "pending" }, now, "h24")).toBeNull();
  });
  it("says when it was last seen", () => {
    expect(
      staleNote(
        "claude",
        { state: "stale", reading, since: 0, cause: { kind: "quiet" } },
        now,
        "h24",
      ),
    ).toBe("Last seen 2h 03m ago · updates when Claude is used");
    expect(
      staleNote(
        "codex",
        {
          state: "stale",
          reading,
          since: 0,
          cause: { kind: "failing", reason: "offline" },
        },
        now,
        "h24",
      ),
    ).toBe("Can't reach Codex · last seen 2h 03m ago");
  });
});

describe("detail-screen words", () => {
  it("says where each provider's numbers come from", () => {
    expect(sourceLine("claude")).toContain("gateway log");
    expect(sourceLine("codex")).toContain("app-server");
    expect(sourceLine("deepseek")).toContain("DeepSeek balance");
    for (const none of ["gemini", "antigravity", "grok"] as const) {
      expect(sourceLine(none)).toBe("Source: none yet");
    }
  });

  it("offers Retry only where trying again can help", () => {
    for (const reason of [
      "offline",
      "timeout",
      "rate_limited",
      "server_error",
      "protocol",
      "io",
      "internal",
    ] as const) {
      expect(retryable(reason)).toBe(true);
    }
    for (const reason of [
      "not_installed",
      "not_logged_in",
      "unauthorized",
    ] as const) {
      expect(retryable(reason)).toBe(false);
    }
  });

  it("writes the first-run line for every state", () => {
    const reading = { observed_at: 0, plan: null, meters: [] };
    const line = (
      usage: Parameters<typeof firstRunLine>[1],
      provider: "claude" | "codex" | "deepseek" = "claude",
    ) => firstRunLine(provider, usage).text;
    expect(line({ state: "known", reading })).toBe("Claude · found");
    expect(
      line({ state: "stale", reading, since: 0, cause: { kind: "quiet" } }),
    ).toBe("Claude · found");
    expect(line({ state: "pending" })).toBe("Claude · checking");
    expect(
      line({ state: "not_configured", setup: "add_deepseek_key" }, "deepseek"),
    ).toBe("DeepSeek · add a key");
    expect(line({ state: "not_configured", setup: "use_gateway" })).toBe(
      "Claude · run jev-claude once",
    );
    expect(
      line({ state: "unavailable", reason: "not_installed" }, "codex"),
    ).toBe("Codex · not installed");
    expect(
      line({ state: "unavailable", reason: "not_logged_in" }, "codex"),
    ).toBe("Codex · not signed in");
    expect(
      line({ state: "unavailable", reason: "unauthorized" }, "codex"),
    ).toBe("Codex · not signed in");
    expect(line({ state: "unavailable", reason: "offline" }, "codex")).toBe(
      "Codex · can't reach it yet",
    );
    expect(line({ state: "unsupported" })).toBe("Claude · no usage source yet");
    expect(firstRunLine("claude", { state: "known", reading }).glyph).toBe("✓");
  });
});
