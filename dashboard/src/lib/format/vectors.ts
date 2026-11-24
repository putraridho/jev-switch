// Typed access to dashboard/fixtures/format-vectors.json for the `format` tests
// (docs/dashboard/DESIGN.md 9.9). JSON enters TypeScript only as an imported module, whose
// string fields are plain `string`; the closed vocabularies are narrowed here, never
// asserted. A value outside its vocabulary yields `null`, which the tests treat as a
// broken fixture.

import raw from "../../../fixtures/format-vectors.json" with { type: "json" };
import type { Currency } from "../ipc/bindings/Currency";
import type { HourCycle } from "../ipc/bindings/HourCycle";
import type { WindowKind } from "../ipc/bindings/WindowKind";

/** The fixed zone every time vector is written in. */
export type VectorZone = { name: string; utcOffsetMinutes: number };

export type PercentVector = { id: string; bp: number; expect: string };
export type CountdownVector = {
  id: string;
  resetsAtS: number;
  nowMs: number;
  expect: string | null;
};
export type AbsoluteVector = {
  id: string;
  resetsAtS: number;
  nowMs: number;
  hour: HourCycle;
  expect: string;
};
export type AgeVector = {
  id: string;
  atMs: number;
  nowMs: number;
  hour: HourCycle;
  expect: string;
};
export type MoneyVector = {
  id: string;
  micros: number;
  currency: Currency;
  expect: string;
};
export type WindowNameVector = {
  id: string;
  window: WindowKind;
  expect: string;
};

/** Every vector, grouped by the function it tests. */
export type FormatVectors = {
  zone: VectorZone;
  percent: PercentVector[];
  percentDetail: PercentVector[];
  countdown: CountdownVector[];
  absolute: AbsoluteVector[];
  age: AgeVector[];
  money: MoneyVector[];
  windowName: WindowNameVector[];
};

function hourCycle(value: string): HourCycle | null {
  return value === "auto" || value === "h12" || value === "h24" ? value : null;
}

function currency(value: string): Currency | null {
  return value === "usd" || value === "cny" || value === "other" ? value : null;
}

function windowKind(value: {
  kind: string;
  minutes: number | null;
  label: string | null;
}): WindowKind | null {
  switch (value.kind) {
    case "five_hour":
      return { kind: "five_hour" };
    case "seven_day":
      return { kind: "seven_day" };
    case "monthly":
      return { kind: "monthly" };
    case "rolling":
      return value.minutes === null
        ? null
        : { kind: "rolling", minutes: value.minutes };
    case "named":
      return value.label === null
        ? null
        : { kind: "named", label: value.label };
    default:
      return null;
  }
}

function present<T>(items: readonly (T | null)[]): T[] {
  const kept: T[] = [];
  for (const item of items) {
    if (item !== null) {
      kept.push(item);
    }
  }
  return kept;
}

/** The number of cases in the file, so a test can see that none was dropped by narrowing. */
export const rawCaseCount: number =
  raw.percent.length +
  raw.percent_detail.length +
  raw.countdown.length +
  raw.absolute.length +
  raw.age.length +
  raw.money.length +
  raw.window_name.length;

/** The vectors, typed. */
export const vectors: FormatVectors = {
  zone: {
    name: raw.zone.name,
    utcOffsetMinutes: raw.zone.utc_offset_minutes,
  },
  percent: raw.percent.map((c) => ({ id: c.id, bp: c.bp, expect: c.expect })),
  percentDetail: raw.percent_detail.map((c) => ({
    id: c.id,
    bp: c.bp,
    expect: c.expect,
  })),
  countdown: raw.countdown.map((c) => ({
    id: c.id,
    resetsAtS: c.resets_at_s,
    nowMs: c.now_ms,
    expect: c.expect,
  })),
  absolute: present(
    raw.absolute.map((c) => {
      const hour = hourCycle(c.hour);
      return hour === null
        ? null
        : {
            id: c.id,
            resetsAtS: c.resets_at_s,
            nowMs: c.now_ms,
            hour,
            expect: c.expect,
          };
    }),
  ),
  age: present(
    raw.age.map((c) => {
      const hour = hourCycle(c.hour);
      return hour === null
        ? null
        : {
            id: c.id,
            atMs: c.at_ms,
            nowMs: c.now_ms,
            hour,
            expect: c.expect,
          };
    }),
  ),
  money: present(
    raw.money.map((c) => {
      const unit = currency(c.currency);
      return unit === null
        ? null
        : { id: c.id, micros: c.micros, currency: unit, expect: c.expect };
    }),
  ),
  windowName: present(
    raw.window_name.map((c) => {
      const window = windowKind(c.window);
      return window === null ? null : { id: c.id, window, expect: c.expect };
    }),
  ),
};
