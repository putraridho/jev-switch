import { describe, expect, it } from "vitest";
import { fixtures } from "./ipc/fixtures";
import type { ProviderView } from "./ipc/bindings/ProviderView";
import type { Snapshot } from "./ipc/bindings/Snapshot";
import { arrange, worstSeverity } from "./overview";

const names = (snapshot: Snapshot) =>
  arrange(snapshot).cards.map((card) => card.provider);

describe("overview arrangement", () => {
  it("keeps the fixed provider order when nothing is raised", () => {
    expect(names(fixtures.known.snapshot)).toEqual([
      "claude",
      "codex",
      "deepseek",
    ]);
  });

  it("puts the most urgent card first, then the fixed order", () => {
    const base = fixtures.critical.snapshot;
    const swapped: Snapshot = {
      ...base,
      providers: [...base.providers].reverse(),
    };
    expect(names(swapped)).toEqual(["claude", "codex", "deepseek"]);
    expect(
      names({
        ...fixtures.known.snapshot,
        providers: [
          ...fixtures.warn.snapshot.providers.filter(
            (p) => p.provider === "claude",
          ),
          ...fixtures.known.snapshot.providers.filter(
            (p) => p.provider !== "claude",
          ),
        ],
      })[0],
    ).toBe("claude");
  });

  it("splits the providers with no usage source into one quiet group", () => {
    expect(
      arrange(fixtures.known.snapshot).unsupported.map((p) => p.provider),
    ).toEqual(["gemini", "antigravity", "grok"]);
  });

  it("leaves hidden providers out", () => {
    const hidden: Snapshot = {
      ...fixtures.known.snapshot,
      providers: fixtures.known.snapshot.providers.map((p) =>
        p.provider === "claude" ? { ...p, enabled: false } : p,
      ),
    };
    expect(names(hidden)).toEqual(["codex", "deepseek"]);
  });

  it("takes the worst level among a provider's meters", () => {
    const claude = (name: "warn" | "critical" | "pending"): ProviderView => {
      const found = fixtures[name].snapshot.providers.find(
        (p) => p.provider === "claude",
      );
      if (found === undefined) throw new Error("fixture provider missing");
      return found;
    };
    expect(worstSeverity(claude("critical"))).toBe("critical");
    expect(worstSeverity(claude("warn"))).toBe("warn");
    expect(worstSeverity(claude("pending"))).toBe("unknown");
  });
});
