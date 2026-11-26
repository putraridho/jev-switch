// The geometry of the sparkline (S48): a 96 by 24 trace, 2 left, 6 right, 3 top and bottom.

import type { HistorySeries } from "../../ipc/bindings/HistorySeries";

export const SPARK = {
  width: 96,
  height: 24,
  left: 2,
  right: 90,
  top: 3,
  bottom: 21,
} as const;

export interface Spot {
  x: number;
  y: number;
}

export interface Trace {
  /** Path data for the part below the warning mark. */
  ink: string;
  /** Path data for the part at or past it. */
  alert: string;
  /** The endpoint dot. */
  end: Spot & { alert: boolean };
  /** The y of the dashed warning guide, when the trace reaches it. */
  guide: number | null;
}

function at(spot: Spot): string {
  return `${String(Math.round(spot.x * 100) / 100)} ${String(Math.round(spot.y * 100) / 100)}`;
}

function segment(from: Spot, to: Spot): string {
  return `M${at(from)} L${at(to)}`;
}

/**
 * The trace of a series, or `null` when it has no points: the sparkline never invents one.
 * Percentages use the whole 0 to 100 scale; balances use their own range and have no
 * warning mark. A series that never changes sits at mid height, and a hole longer than
 * `gap_s` stays open.
 */
export function buildTrace(
  series: HistorySeries,
  warnPct: number,
): Trace | null {
  const points = series.points;
  const first = points[0];
  const last = points[points.length - 1];
  if (first === undefined || last === undefined) return null;

  const percent = series.unit === "basis_points";
  const values = points.map((point) => point.v);
  const low = Math.min(...values);
  const high = Math.max(...values);
  const flat = low === high;
  const floor = percent ? 0 : low;
  const ceiling = percent ? Math.max(10_000, high) : high;
  const warn = percent ? warnPct * 100 : null;

  const span = last.t - first.t;
  const x = (t: number): number =>
    span === 0
      ? SPARK.right
      : SPARK.left + ((t - first.t) / span) * (SPARK.right - SPARK.left);
  const y = (v: number): number =>
    flat
      ? SPARK.height / 2
      : SPARK.bottom -
        ((v - floor) / (ceiling - floor)) * (SPARK.bottom - SPARK.top);
  const past = (v: number): boolean => warn !== null && v >= warn;

  let ink = "";
  let alert = "";
  const add = (raised: boolean, path: string): void => {
    if (raised) alert += path;
    else ink += path;
  };
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1];
    const b = points[i];
    if (a === undefined || b === undefined || b.t - a.t > series.gap_s)
      continue;
    const from = { x: x(a.t), y: y(a.v) };
    const to = { x: x(b.t), y: y(b.v) };
    if (warn === null || past(a.v) === past(b.v)) {
      add(past(a.v), segment(from, to));
      continue;
    }
    // The line changes colour exactly where it crosses the mark.
    const share = (warn - a.v) / (b.v - a.v);
    const cross = {
      x: from.x + (to.x - from.x) * share,
      y: from.y + (to.y - from.y) * share,
    };
    add(past(a.v), segment(from, cross));
    add(past(b.v), segment(cross, to));
  }

  return {
    ink,
    alert,
    end: { x: x(last.t), y: y(last.v), alert: past(last.v) },
    guide: warn !== null && !flat && past(high) ? y(warn) : null,
  };
}
