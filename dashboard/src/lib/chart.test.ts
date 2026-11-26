import { describe, expect, it } from "vitest";
import {
  buildChart,
  frameFor,
  momentLabel,
  nearest,
  niceStep,
  readout,
  stepIndex,
  summary,
  valueLabel,
  yMax,
} from "./chart";
import { FIXTURE_NOW_MS, fixtures } from "./ipc/fixtures";
import type { HistorySeries } from "./ipc/bindings/HistorySeries";

const series = fixtures.known.history;
const money = (micros: number): string => `¥${(micros / 1e6).toFixed(2)}`;
const nowMs = FIXTURE_NOW_MS;

function build(s: HistorySeries = series, width = 400) {
  return buildChart(
    s,
    { warnBp: 8000, criticalBp: 9500 },
    "day",
    "auto",
    nowMs,
    money,
    width,
  );
}

describe("chart geometry", () => {
  it("frames a container with room for the axis words", () => {
    expect(frameFor(400)).toMatchObject({ width: 400, height: 200, left: 44 });
    expect(frameFor(10).width).toBe(240);
  });

  it("rounds to a nice step", () => {
    expect(niceStep(0)).toBe(1);
    expect(niceStep(0.7)).toBe(1);
    expect(niceStep(1.5)).toBe(2);
    expect(niceStep(3.2)).toBe(5);
    expect(niceStep(7)).toBe(10);
    expect(niceStep(130)).toBe(200);
  });

  it("fixes the usage axis at 100 % and scales money to the data", () => {
    expect(yMax(series)).toBe(10_000);
    const balance: HistorySeries = {
      ...series,
      unit: "micros",
      points: [{ t: series.from, v: 7_000_000, lo: 7_000_000, hi: 9_000_000 }],
    };
    expect(yMax(balance)).toBeGreaterThanOrEqual(9_000_000);
    expect(yMax({ ...balance, points: [] })).toBe(1);
  });

  it("words values by unit", () => {
    expect(valueLabel({ unit: "basis_points" }, 7_250, money)).toBe("73%");
    expect(valueLabel({ unit: "micros" }, 1_500_000, money)).toBe("¥1.50");
  });

  it("builds step lines with a gap and one reset line", () => {
    const model = build();
    // Two runs of points: the three-hour gap breaks the line into two subpaths.
    expect(model.line.match(/M/g)).toHaveLength(2);
    expect(model.area.match(/Z/g)).toHaveLength(2);
    expect(model.line).toContain("H");
    expect(model.line).toContain("V");
    expect(model.band).toBeNull();
    expect(model.resets).toHaveLength(series.resets.length);
    expect(series.resets.length).toBeGreaterThan(1);
    expect(model.thresholds.map((line) => line.kind)).toEqual([
      "critical",
      "warn",
    ]);
    expect(model.thresholds[0]?.label).toBe("critical 95%");
    expect(model.yTicks).toHaveLength(5);
    expect(model.xTicks).toHaveLength(4);
    expect(model.points).toHaveLength(series.points.length);
  });

  it("draws a band when a rollup point has a spread", () => {
    const banded: HistorySeries = {
      ...series,
      points: series.points.map((point, i) =>
        i % 2 === 0
          ? { ...point, lo: point.v - 200, hi: point.v + 200 }
          : point,
      ),
    };
    expect(build(banded).band).not.toBeNull();
  });

  it("draws no thresholds for a balance", () => {
    const model = buildChart(
      { ...series, unit: "micros" },
      null,
      "week",
      "auto",
      nowMs,
      money,
    );
    expect(model.thresholds).toEqual([]);
  });

  it("copes with no points and a flat span", () => {
    const empty = build({ ...series, points: [], resets: [], to: series.from });
    expect(empty.line).toBe("");
    expect(empty.points).toEqual([]);
  });

  it("ignores resets outside the range", () => {
    expect(build({ ...series, resets: [1, series.to + 5] }).resets).toEqual([]);
  });

  it("labels a day by the clock and longer ranges by date", () => {
    expect(momentLabel(Math.round(nowMs / 1000), "day", "h24", nowMs)).toBe(
      "12:33",
    );
    expect(momentLabel(Math.round(nowMs / 1000), "week", "h24", nowMs)).toBe(
      "today 12:33",
    );
  });
});

describe("chart interaction", () => {
  const model = build();

  it("finds the nearest point and none without points", () => {
    expect(nearest([], 10)).toBeNull();
    const second = model.points[1];
    expect(second).toBeDefined();
    expect(nearest(model.points, (second?.x ?? 0) + 0.2)).toBe(1);
  });

  it("steps and clamps the slider", () => {
    expect(stepIndex("ArrowRight", 2, 5)).toBe(3);
    expect(stepIndex("ArrowUp", 4, 5)).toBe(4);
    expect(stepIndex("ArrowLeft", 0, 5)).toBe(0);
    expect(stepIndex("ArrowDown", 3, 5)).toBe(2);
    expect(stepIndex("PageUp", 0, 30)).toBe(10);
    expect(stepIndex("PageDown", 4, 30)).toBe(0);
    expect(stepIndex("Home", 3, 5)).toBe(0);
    expect(stepIndex("End", 3, 5)).toBe(4);
    expect(stepIndex("a", 3, 5)).toBeNull();
  });
});

describe("chart words", () => {
  it("summarises peak, now and resets", () => {
    const text = summary(series, "5-hour", "day", "h24", nowMs, money);
    expect(text).toMatch(/^5-hour usage over the last day: peak \d+% at /);
    expect(text).toContain("now ");
    expect(text).toMatch(/, \d+ resets$/);
  });

  it("pluralises resets and says so when there is no history", () => {
    expect(
      summary(
        { ...series, resets: [series.from + 10, series.from + 20] },
        "5-hour",
        "week",
        "h24",
        nowMs,
        money,
      ),
    ).toContain("2 resets");
    expect(
      summary({ ...series, points: [] }, "5-hour", "day", "h24", nowMs, money),
    ).toBe("5-hour usage over the last day: no history yet");
    expect(
      summary(
        { ...series, resets: [], unit: "micros" },
        "Balance",
        "month",
        "h24",
        nowMs,
        money,
      ),
    ).toContain("Balance balance over the last month");
  });

  it("reads one point as time and value", () => {
    const point = series.points[series.points.length - 1];
    if (point === undefined) throw new Error("fixture has no points");
    expect(readout(series, point, "day", "h24", nowMs, money)).toMatch(
      /^\d{1,2}:\d\d( [AP]M)?, \d+ percent$/,
    );
    expect(
      readout({ ...series, unit: "micros" }, point, "day", "h24", nowMs, money),
    ).toMatch(/, ¥/);
  });
});
