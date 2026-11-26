// The shared format vectors load completely and are well formed (docs/dashboard/TASKS.md
// T00). The assertions of the formatters themselves are T07's.

import { describe, expect, it } from "vitest";
import { rawCaseCount, vectors } from "./vectors";

describe("format vectors", () => {
  const all = [
    ...vectors.percent,
    ...vectors.percentDetail,
    ...vectors.countdown,
    ...vectors.absolute,
    ...vectors.age,
    ...vectors.money,
    ...vectors.windowName,
  ];

  it("has at least 60 cases and loses none to narrowing", () => {
    expect(rawCaseCount).toBeGreaterThanOrEqual(60);
    expect(all.length).toBe(rawCaseCount);
  });

  it("gives every case a unique id", () => {
    const ids = all.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("covers every rule of DESIGN 9.9", () => {
    expect(vectors.percent.length).toBeGreaterThanOrEqual(10);
    expect(vectors.percentDetail.length).toBeGreaterThanOrEqual(4);
    expect(vectors.countdown.some((c) => c.expect === null)).toBe(true);
    expect(vectors.countdown.some((c) => c.expect === "under a minute")).toBe(
      true,
    );
    for (const word of ["today", "tomorrow", "Thu", "2027"]) {
      expect(vectors.absolute.some((c) => c.expect.includes(word))).toBe(true);
    }
    for (const hour of ["h12", "h24"]) {
      expect(vectors.absolute.some((c) => c.hour === hour)).toBe(true);
    }
    for (const word of ["just now", "ago", "yesterday"]) {
      expect(vectors.age.some((c) => c.expect.includes(word))).toBe(true);
    }
    for (const unit of ["usd", "cny", "other"]) {
      expect(vectors.money.some((c) => c.currency === unit)).toBe(true);
    }
    for (const kind of [
      "five_hour",
      "seven_day",
      "monthly",
      "rolling",
      "named",
    ]) {
      expect(vectors.windowName.some((c) => c.window.kind === kind)).toBe(true);
    }
    expect(vectors.zone).toEqual({
      name: "Asia/Jakarta",
      utcOffsetMinutes: 420,
    });
  });
});
