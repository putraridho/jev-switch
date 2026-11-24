// Numbers, times and names for display (docs/dashboard/DESIGN.md 9.9). Pure; tested against
// dashboard/fixtures/format-vectors.json. The signatures are final; T07 writes the bodies.

import type { Currency } from "../ipc/bindings/Currency";
import type { HourCycle } from "../ipc/bindings/HourCycle";
import type { MeterView } from "../ipc/bindings/MeterView";
import type { WindowKind } from "../ipc/bindings/WindowKind";

/** "79%", "<1%", "104%": whole, rounded down. */
export function percent(_bp: number): string {
  return "";
}

/** "3d 4h", "2h 14m", "38m", "under a minute"; `null` when the time has passed. */
export function countdown(_resetsAtS: number, _nowMs: number): string | null {
  return null;
}

/** "today 14:00", "tomorrow 02:00", "Thu 02:00", "16 Dec 08:37". */
export function absolute(
  _resetsAtS: number,
  _nowMs: number,
  _hour: HourCycle,
  _zone?: string,
): string {
  return "";
}

/** "just now", "5m ago", "2h 05m ago", "yesterday 21:10", then a date. */
export function age(_atMs: number, _nowMs: number, _hour: HourCycle): string {
  return "";
}

/** A balance: currency style with two decimals; `other` is a plain number. */
export function money(_micros: number, _currency: Currency): string {
  return "";
}

/** "5-hour", "7-day", "30-day", "N-hour"/"N-day", or the raw label. */
export function windowName(_w: WindowKind): string {
  return "";
}

/** The full level-1 sentence of a meter, used as its `aria-valuetext`. */
export function meterValueText(
  _m: MeterView,
  _nowMs: number,
  _hour: HourCycle,
): string {
  return "";
}
