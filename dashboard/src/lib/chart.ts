// History chart geometry and words (DESIGN 9.4). Pure: a `HistorySeries` goes in, SVG path
// strings, tick positions and the spoken summary come out. No chart library.

import { absolute, timeOfDay } from "./format";
import type { HistoryPoint } from "./ipc/bindings/HistoryPoint";
import type { HistoryRange } from "./ipc/bindings/HistoryRange";
import type { HistorySeries } from "./ipc/bindings/HistorySeries";
import type { HourCycle } from "./ipc/bindings/HourCycle";

/** The drawing box and the plot inside it, in SVG user units. */
export type Frame = {
  width: number;
  height: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
};

/** The frame for a container `width` px wide: room on the left for the axis words and below
 * for the times. The chart is drawn at 1 user unit = 1 CSS px so its words stay readable. */
export function frameFor(width: number): Frame {
  return {
    width: Math.max(240, Math.round(width)),
    height: 200,
    left: 44,
    right: 12,
    top: 12,
    bottom: 28,
  };
}

/** A horizontal threshold line with its word. */
export type ThresholdLine = {
  y: number;
  label: string;
  kind: "warn" | "critical";
};
/** An axis tick with its word. */
export type Tick = { at: number; label: string };
/** A point in plot coordinates, with the data it came from. */
export type Placed = { x: number; y: number; point: HistoryPoint };

/** Everything the component draws. */
export type ChartModel = {
  frame: Frame;
  /** Step lines, one subpath per run of points without a gap. */
  line: string;
  /** The soft fill under each run. */
  area: string;
  /** The lo..hi band of rollup ranges, or `null` when every point is exact. */
  band: string | null;
  yTicks: Tick[];
  xTicks: Tick[];
  resets: number[];
  thresholds: ThresholdLine[];
  points: Placed[];
};

/** The meter's thresholds in basis points, or `null` for a balance. */
export type Thresholds = { warnBp: number; criticalBp: number } | null;

function round(n: number): number {
  return Math.round(n * 10) / 10;
}

/** "5-hour usage over the last day" needs the range as words. */
export const rangeWords: Record<HistoryRange, string> = {
  day: "the last day",
  week: "the last week",
  month: "the last month",
  quarter: "the last three months",
};

/** The range control's labels. */
export const rangeNames: Record<HistoryRange, string> = {
  day: "Day",
  week: "Week",
  month: "Month",
  quarter: "Quarter",
};

/** A step between two "nice" numbers at or above `raw`. */
export function niceStep(raw: number): number {
  if (!(raw > 0)) return 1;
  const power = 10 ** Math.floor(Math.log10(raw));
  const unit = raw / power;
  const nice = unit <= 1 ? 1 : unit <= 2 ? 2 : unit <= 5 ? 5 : 10;
  return nice * power;
}

/** The top of the y axis: 100 % for usage, the next nice step above the data for money. */
export function yMax(series: HistorySeries): number {
  if (series.unit === "basis_points") return 10_000;
  let top = 0;
  for (const point of series.points) top = Math.max(top, point.hi, point.v);
  return top <= 0 ? 1 : Math.ceil(top / niceStep(top / 4)) * niceStep(top / 4);
}

/** The words for a value on the axis or in a readout. */
export function valueLabel(
  series: Pick<HistorySeries, "unit">,
  value: number,
  money: (micros: number) => string,
): string {
  return series.unit === "basis_points"
    ? `${String(Math.round(value / 100))}%`
    : money(value);
}

/** The words for a moment on the x axis: a time of day for a day, else a date. */
export function momentLabel(
  seconds: number,
  range: HistoryRange,
  hour: HourCycle,
  nowMs: number,
): string {
  return range === "day"
    ? timeOfDay(seconds * 1000, hour)
    : absolute(seconds, nowMs, hour);
}

/** Builds the drawing for `series`. `money` words a currency value for the axis. */
export function buildChart(
  series: HistorySeries,
  thresholds: Thresholds,
  range: HistoryRange,
  hour: HourCycle,
  nowMs: number,
  money: (micros: number) => string,
  width = 480,
): ChartModel {
  const frame = frameFor(width);
  const plotW = frame.width - frame.left - frame.right;
  const plotH = frame.height - frame.top - frame.bottom;
  const span = Math.max(1, series.to - series.from);
  const top = yMax(series);
  const x = (t: number): number =>
    round(frame.left + ((t - series.from) / span) * plotW);
  const y = (v: number): number =>
    round(frame.top + plotH - (Math.min(Math.max(v, 0), top) / top) * plotH);
  const baseline = round(frame.top + plotH);

  const points: Placed[] = series.points.map((point) => ({
    x: x(point.t),
    y: y(point.v),
    point,
  }));

  // Runs of points: a gap longer than `gap_s` is drawn as a gap, never interpolated.
  const runs: Placed[][] = [];
  for (const placed of points) {
    const run = runs[runs.length - 1];
    const last = run?.[run.length - 1];
    if (run !== undefined && last !== undefined) {
      const gap =
        series.gap_s > 0 && placed.point.t - last.point.t > series.gap_s;
      if (!gap) {
        run.push(placed);
        continue;
      }
    }
    runs.push([placed]);
  }

  const lineParts: string[] = [];
  const areaParts: string[] = [];
  const bandParts: string[] = [];
  let anyBand = false;
  for (const run of runs) {
    const first = run[0];
    const last = run[run.length - 1];
    if (first === undefined || last === undefined) continue;
    // Usage is piecewise constant between samples: hold the value, then step.
    let d = `M${String(first.x)} ${String(first.y)}`;
    for (let i = 1; i < run.length; i += 1) {
      const before = run[i - 1];
      const here = run[i];
      if (before === undefined || here === undefined) continue;
      d += `H${String(here.x)}V${String(here.y)}`;
    }
    lineParts.push(d);
    areaParts.push(`${d}V${String(baseline)}H${String(first.x)}Z`);
    if (run.some((p) => p.point.lo !== p.point.hi)) {
      anyBand = true;
      let upper = `M${String(first.x)} ${String(y(first.point.hi))}`;
      for (let i = 1; i < run.length; i += 1) {
        const here = run[i];
        if (here === undefined) continue;
        upper += `H${String(here.x)}V${String(y(here.point.hi))}`;
      }
      let lower = "";
      for (let i = run.length - 1; i >= 0; i -= 1) {
        const here = run[i];
        if (here === undefined) continue;
        lower += `${i === run.length - 1 ? "L" : "H"}${String(here.x)}V${String(y(here.point.lo))}`;
      }
      bandParts.push(`${upper}${lower}Z`);
    }
  }

  const steps = 4;
  const yTicks: Tick[] = [];
  for (let i = 0; i <= steps; i += 1) {
    const value = (top / steps) * i;
    yTicks.push({
      at: y(value),
      label: valueLabel(series, value, money),
    });
  }

  const xTicks: Tick[] = [];
  for (let i = 0; i <= 3; i += 1) {
    const seconds = series.from + (span / 3) * i;
    xTicks.push({
      at: x(seconds),
      label: momentLabel(Math.round(seconds), range, hour, nowMs),
    });
  }

  const resets = series.resets
    .filter((t) => t >= series.from && t <= series.to)
    .map(x);

  const lines: ThresholdLine[] =
    thresholds === null
      ? []
      : [
          {
            y: y(thresholds.criticalBp),
            label: `critical ${String(Math.round(thresholds.criticalBp / 100))}%`,
            kind: "critical",
          },
          {
            y: y(thresholds.warnBp),
            label: `warn ${String(Math.round(thresholds.warnBp / 100))}%`,
            kind: "warn",
          },
        ];

  return {
    frame,
    line: lineParts.join(""),
    area: areaParts.join(""),
    band: anyBand ? bandParts.join("") : null,
    yTicks,
    xTicks,
    resets,
    thresholds: lines,
    points,
  };
}

/** The point whose x is nearest `px` (plot coordinates), or `null` with no points. */
export function nearest(points: readonly Placed[], px: number): number | null {
  let best: number | null = null;
  let distance = Infinity;
  points.forEach((placed, index) => {
    const d = Math.abs(placed.x - px);
    if (d < distance) {
      distance = d;
      best = index;
    }
  });
  return best;
}

/** The next index for a slider key, or `null` when the key does nothing. */
export function stepIndex(
  key: string,
  current: number,
  count: number,
): number | null {
  const last = count - 1;
  switch (key) {
    case "ArrowLeft":
    case "ArrowDown":
      return Math.max(0, current - 1);
    case "ArrowRight":
    case "ArrowUp":
      return Math.min(last, current + 1);
    case "PageDown":
      return Math.max(0, current - 10);
    case "PageUp":
      return Math.min(last, current + 10);
    case "Home":
      return 0;
    case "End":
      return last;
    default:
      return null;
  }
}

/** "5-hour usage over the last day: peak 72% at 09:40, now 50%, 3 resets". */
export function summary(
  series: HistorySeries,
  name: string,
  range: HistoryRange,
  hour: HourCycle,
  nowMs: number,
  money: (micros: number) => string,
): string {
  const lead = `${name} ${series.unit === "basis_points" ? "usage" : "balance"} over ${rangeWords[range]}`;
  const lastPoint = series.points[series.points.length - 1];
  if (lastPoint === undefined) return `${lead}: no history yet`;
  let peak = lastPoint;
  for (const point of series.points) if (point.hi > peak.hi) peak = point;
  const resets = series.resets.filter(
    (t) => t >= series.from && t <= series.to,
  ).length;
  const resetWords =
    resets === 0 ? "" : `, ${String(resets)} reset${resets === 1 ? "" : "s"}`;
  return `${lead}: peak ${valueLabel(series, peak.hi, money)} at ${momentLabel(peak.t, range, hour, nowMs)}, now ${valueLabel(series, lastPoint.v, money)}${resetWords}`;
}

/** The slider's value text and the readout: "09:40, 72 percent". */
export function readout(
  series: HistorySeries,
  point: HistoryPoint,
  range: HistoryRange,
  hour: HourCycle,
  nowMs: number,
  money: (micros: number) => string,
): string {
  const when = momentLabel(point.t, range, hour, nowMs);
  const value =
    series.unit === "basis_points"
      ? `${String(Math.round(point.v / 100))} percent`
      : money(point.v);
  return `${when}, ${value}`;
}
