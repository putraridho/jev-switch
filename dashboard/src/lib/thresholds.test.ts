import { describe, expect, it } from "vitest";
import { checkCritical, checkWarn } from "./thresholds";

describe("threshold fields", () => {
  it("accepts a warn level below critical", () => {
    expect(checkWarn(70, 9500)).toEqual({ ok: true, bp: 7000 });
  });

  it("explains a warn level at or above critical", () => {
    expect(checkWarn(95, 9500)).toEqual({
      ok: false,
      message: "Warn must be below critical (95)",
    });
  });

  it("accepts a critical level above warn and explains one below", () => {
    expect(checkCritical(90, 8000)).toEqual({ ok: true, bp: 9000 });
    expect(checkCritical(80, 8000)).toEqual({
      ok: false,
      message: "Critical must be above warn (80)",
    });
  });

  it("rejects what is not a whole number from 50 to 100", () => {
    for (const bad of [49, 101, 70.5, Number.NaN]) {
      expect(checkWarn(bad, 9500).ok).toBe(false);
      expect(checkCritical(bad, 5000).ok).toBe(false);
    }
  });
});
