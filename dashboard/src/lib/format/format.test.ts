// The formatters against the shared vectors (docs/dashboard/DESIGN.md 9.9), then the
// sentences built from them.

import { describe, expect, it } from "vitest";
import type { MeterView } from "../ipc/bindings/MeterView";
import {
  absolute,
  absoluteSpoken,
  age,
  countdown,
  countdownSpoken,
  meterValueText,
  money,
  percent,
  percentDetail,
  timeOfDay,
  windowName,
} from "./index";
import { vectors } from "./vectors";

const zone = vectors.zone.name;

describe("format vectors", () => {
  for (const c of vectors.percent) {
    it(c.id, () => {
      expect(percent(c.bp)).toBe(c.expect);
    });
  }
  for (const c of vectors.percentDetail) {
    it(c.id, () => {
      expect(percentDetail(c.bp)).toBe(c.expect);
    });
  }
  for (const c of vectors.countdown) {
    it(c.id, () => {
      expect(countdown(c.resetsAtS, c.nowMs)).toBe(c.expect);
    });
  }
  for (const c of vectors.absolute) {
    it(c.id, () => {
      expect(absolute(c.resetsAtS, c.nowMs, c.hour, zone)).toBe(c.expect);
    });
  }
  for (const c of vectors.age) {
    it(c.id, () => {
      expect(age(c.atMs, c.nowMs, c.hour, zone)).toBe(c.expect);
    });
  }
  for (const c of vectors.money) {
    it(c.id, () => {
      expect(money(c.micros, c.currency)).toBe(c.expect);
    });
  }
  for (const c of vectors.windowName) {
    it(c.id, () => {
      expect(windowName(c.window)).toBe(c.expect);
    });
  }
});

describe("format edges", () => {
  it("clamps a negative percentage to zero", () => {
    expect(percent(-5)).toBe("0%");
    expect(percentDetail(-5)).toBe("0.0 %");
  });

  it("never prints a negative zero amount", () => {
    expect(money(-4000, "usd")).toBe("$0.00");
  });

  it("signs a negative amount in other currencies", () => {
    expect(money(-3_500_000, "cny")).toBe("-CN¥3.50");
    expect(money(-3_500_000, "other")).toBe("-3.50 balance");
  });

  it("resolves the hour cycle from the locale for auto", () => {
    expect(absolute(1797231600, 1797226380000, "auto", zone)).toMatch(
      /^today (14:00|2:00 PM)$/,
    );
  });

  it("falls back to a date for an instant in the past", () => {
    expect(absolute(1797000000, 1797226380000, "h24", zone)).toMatch(
      /^\d+ \w{3} \d{2}:\d{2}$/,
    );
  });

  it("formats the zone-less local time", () => {
    expect(timeOfDay(1797226380000, "h24", zone)).toBe("12:33");
    expect(timeOfDay(1797226380000, "h12", zone)).toBe("12:33 PM");
    expect(timeOfDay(1797226380000, "h24")).toMatch(/^\d{2}:\d{2}$/);
  });
});

describe("spoken forms", () => {
  const now = 1797226380000;

  it("reads a countdown in words", () => {
    expect(countdownSpoken(1797234420, now)).toBe("2 hours 14 minutes");
    expect(countdownSpoken(1797228680, now)).toBe("38 minutes");
    expect(countdownSpoken(1797226440, now)).toBe("1 minute");
    expect(countdownSpoken(1797226439, now)).toBe("under a minute");
    expect(countdownSpoken(1797501780, now)).toBe("3 days 4 hours");
    expect(countdownSpoken(1797226380, now)).toBeNull();
  });

  it("puts an at before the time of day", () => {
    expect(absoluteSpoken(1797231600, now, "h24", zone)).toBe("today at 14:00");
    expect(absoluteSpoken(1797231600, now, "h12", zone)).toBe(
      "today at 2:00 PM",
    );
    expect(absoluteSpoken(1798421820, now, "h24", zone)).toBe(
      "28 Dec at 08:37",
    );
  });
});

describe("meterValueText", () => {
  const now = 1797226380000;
  const window = (
    patch: Partial<Extract<MeterView, { kind: "window" }>>,
  ): MeterView => ({
    kind: "window",
    id: "w:5h",
    window: { kind: "five_hour" },
    used_bp: 5000,
    resets_at: 1797234420,
    status: "allowed",
    severity: "ok",
    phase: "current",
    ...patch,
  });

  it("carries the whole level-1 detail", () => {
    expect(meterValueText(window({}), now, "h24", now - 60_000)).toMatch(
      /^50\.0 percent used, resets in 2 hours 14 minutes, (today|tomorrow|\w{3}) at \d{1,2}:\d{2}.*, status allowed, seen at \d{2}:\d{2}/,
    );
  });

  it("leaves the seen clause out without an observation time", () => {
    expect(meterValueText(window({}), now, "h24")).not.toContain("seen");
  });

  it("says a rejected window has reached its limit", () => {
    expect(
      meterValueText(window({ status: "rejected" }), now, "h24"),
    ).toContain("status limit reached");
  });

  it("invents no number for an elapsed window", () => {
    const text = meterValueText(
      window({ phase: "elapsed", resets_at: 1797226000 }),
      now,
      "h24",
    );
    expect(text).toContain("window elapsed");
    expect(text).toContain("waiting for your next request");
    expect(text).not.toContain("percent");
  });

  it("treats a reset in the past as elapsed", () => {
    expect(
      meterValueText(window({ resets_at: 1797000000 }), now, "h24"),
    ).toContain("window elapsed");
  });

  it("handles a missing reset time", () => {
    expect(meterValueText(window({ resets_at: null }), now, "h24")).toBe(
      "50.0 percent used, reset time unavailable, status allowed",
    );
  });

  it("says no data for an unknown level", () => {
    expect(meterValueText(window({ severity: "unknown" }), now, "h24")).toBe(
      "No data",
    );
  });

  it("describes a balance", () => {
    const balance: MeterView = {
      kind: "balance",
      id: "b:cny",
      currency: "cny",
      total_micros: 110_000_000,
      granted_micros: null,
      topped_up_micros: null,
      usable: true,
      severity: "ok",
    };
    expect(meterValueText(balance, now, "h24", now)).toMatch(
      /^CN¥110\.00, available, seen at \d{2}:\d{2}$/,
    );
    expect(meterValueText({ ...balance, usable: false }, now, "h24")).toBe(
      "CN¥110.00, not usable",
    );
  });
});
