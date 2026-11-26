// Numbers, times and names for display (docs/dashboard/DESIGN.md 9.9). Pure; tested against
// dashboard/fixtures/format-vectors.json, which the Rust `text` module shares.

import type { Currency } from "../ipc/bindings/Currency";
import type { HourCycle } from "../ipc/bindings/HourCycle";
import type { LimitStatus } from "../ipc/bindings/LimitStatus";
import type { MeterView } from "../ipc/bindings/MeterView";
import type { WindowKind } from "../ipc/bindings/WindowKind";

/** A window meter: the one kind with a reset time and a level. */
export type WindowMeterView = Extract<MeterView, { kind: "window" }>;

const MONTHS: readonly string[] = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];
const WEEKDAYS: readonly string[] = [
  "Sun",
  "Mon",
  "Tue",
  "Wed",
  "Thu",
  "Fri",
  "Sat",
];
const MS_PER_DAY = 86_400_000;

/** A moment broken into local calendar fields. */
type Local = {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  /** Whole days since 1970-01-01 of the local date. */
  dayNumber: number;
};

function field(
  parts: readonly Intl.DateTimeFormatPart[],
  type: string,
): number {
  const part = parts.find((item) => item.type === type);
  return part === undefined ? 0 : Number(part.value);
}

function localParts(ms: number, zone: string | undefined): Local {
  const formatter = new Intl.DateTimeFormat("en-US", {
    timeZone: zone,
    hourCycle: "h23",
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "numeric",
  });
  const parts = formatter.formatToParts(new Date(ms));
  const year = field(parts, "year");
  const month = field(parts, "month");
  const day = field(parts, "day");
  return {
    year,
    month,
    day,
    hour: field(parts, "hour") % 24,
    minute: field(parts, "minute"),
    dayNumber: Math.floor(Date.UTC(year, month - 1, day) / MS_PER_DAY),
  };
}

/** Whether the clock shows AM/PM: the setting, or the locale's habit for `auto`. */
function twelveHour(hour: HourCycle): boolean {
  if (hour === "h12") return true;
  if (hour === "h24") return false;
  const cycle = new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
  }).resolvedOptions().hourCycle;
  return cycle === "h11" || cycle === "h12";
}

function two(n: number): string {
  return n < 10 ? `0${String(n)}` : String(n);
}

function clock(local: Local, hour: HourCycle): string {
  if (!twelveHour(hour)) return `${two(local.hour)}:${two(local.minute)}`;
  const suffix = local.hour < 12 ? "AM" : "PM";
  const h = local.hour % 12 === 0 ? 12 : local.hour % 12;
  return `${String(h)}:${two(local.minute)} ${suffix}`;
}

function dateWords(local: Local, now: Local): string {
  const year = local.year === now.year ? "" : ` ${String(local.year)}`;
  const month = MONTHS[local.month - 1] ?? "";
  return `${String(local.day)} ${month}${year}`;
}

/** "79%", "<1%", "104%": whole, rounded down. */
export function percent(bp: number): string {
  if (bp <= 0) return "0%";
  if (bp < 100) return "<1%";
  return `${String(Math.floor(bp / 100))}%`;
}

/** "79.9 %": one decimal, cut, a space before the sign (level 1). */
export function percentDetail(bp: number): string {
  const tenths = Math.floor(Math.max(0, bp) / 10);
  return `${String(Math.floor(tenths / 10))}.${String(tenths % 10)} %`;
}

/** "3d 4h", "2h 14m", "38m", "under a minute"; `null` when the time has passed. */
export function countdown(resetsAtS: number, nowMs: number): string | null {
  const remainingMs = resetsAtS * 1000 - nowMs;
  if (remainingMs <= 0) return null;
  const seconds = Math.floor(remainingMs / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours >= 48) {
    return `${String(Math.floor(hours / 24))}d ${String(hours % 24)}h`;
  }
  if (hours >= 1) return `${String(hours)}h ${two(minutes % 60)}m`;
  if (minutes >= 1) return `${String(minutes)}m`;
  return "under a minute";
}

/** The same countdown in words a screen reader reads well: "2 hours 14 minutes". */
export function countdownSpoken(
  resetsAtS: number,
  nowMs: number,
): string | null {
  const remainingMs = resetsAtS * 1000 - nowMs;
  if (remainingMs <= 0) return null;
  const minutes = Math.floor(remainingMs / 60_000);
  const hours = Math.floor(minutes / 60);
  const unit = (n: number, word: string): string =>
    `${String(n)} ${word}${n === 1 ? "" : "s"}`;
  if (hours >= 48) {
    return `${unit(Math.floor(hours / 24), "day")} ${unit(hours % 24, "hour")}`;
  }
  if (hours >= 1)
    return `${unit(hours, "hour")} ${unit(minutes % 60, "minute")}`;
  if (minutes >= 1) return unit(minutes, "minute");
  return "under a minute";
}

/** "today 14:00", "tomorrow 02:00", "Thu 02:00", "16 Dec 08:37". */
export function absolute(
  resetsAtS: number,
  nowMs: number,
  hour: HourCycle,
  zone?: string,
): string {
  const at = localParts(resetsAtS * 1000, zone);
  const now = localParts(nowMs, zone);
  const time = clock(at, hour);
  const days = at.dayNumber - now.dayNumber;
  if (days === 0) return `today ${time}`;
  if (days === 1) return `tomorrow ${time}`;
  if (days >= 2 && days <= 6) {
    return `${WEEKDAYS[(at.dayNumber + 4) % 7] ?? ""} ${time}`;
  }
  return `${dateWords(at, now)} ${time}`;
}

/** `absolute`, with "at" before the time: "today at 14:00". */
export function absoluteSpoken(
  resetsAtS: number,
  nowMs: number,
  hour: HourCycle,
  zone?: string,
): string {
  return absolute(resetsAtS, nowMs, hour, zone).replace(
    /\s(\d{1,2}:\d{2}(?: [AP]M)?)$/,
    " at $1",
  );
}

/** The wall-clock time of an instant: "12:33" or "12:33 PM". */
export function timeOfDay(
  atMs: number,
  hour: HourCycle,
  zone?: string,
): string {
  return clock(localParts(atMs, zone), hour);
}

/** "just now", "5m ago", "2h 05m ago", "yesterday 21:10", then a date. */
export function age(
  atMs: number,
  nowMs: number,
  hour: HourCycle,
  zone?: string,
): string {
  const delta = nowMs - atMs;
  if (delta < 60_000) return "just now";
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 60) return `${String(minutes)}m ago`;
  const at = localParts(atMs, zone);
  const now = localParts(nowMs, zone);
  const days = now.dayNumber - at.dayNumber;
  if (days === 0) {
    return `${String(Math.floor(minutes / 60))}h ${two(minutes % 60)}m ago`;
  }
  const time = clock(at, hour);
  if (days === 1) return `yesterday ${time}`;
  return `${dateWords(at, now)} ${time}`;
}

/** A balance: currency style with two decimals; `other` is a plain number. */
export function money(micros: number, currency: Currency): string {
  const cents = Math.round(Math.abs(micros) / 10_000);
  const whole = new Intl.NumberFormat("en-US").format(Math.floor(cents / 100));
  const amount = `${whole}.${two(cents % 100)}`;
  const sign = micros < 0 && cents > 0 ? "-" : "";
  if (currency === "usd") return `${sign}$${amount}`;
  if (currency === "cny") return `${sign}CN¥${amount}`;
  return `${sign}${amount} balance`;
}

/** The words for the windows the app names itself (the Antigravity buckets); `text.rs` has
 * the same four. */
const namedWindows: ReadonlyMap<string, string> = new Map([
  ["gemini_5h", "Gemini 5-hour"],
  ["gemini_7d", "Gemini 7-day"],
  ["claude_gpt_5h", "Claude/GPT 5-hour"],
  ["claude_gpt_7d", "Claude/GPT 7-day"],
]);

/** "5-hour", "7-day", "30-day", "N-hour"/"N-day", or the raw label. */
export function windowName(w: WindowKind): string {
  switch (w.kind) {
    case "five_hour":
      return "5-hour";
    case "seven_day":
      return "7-day";
    case "monthly":
      return "30-day";
    case "rolling":
      return w.minutes % 1440 === 0
        ? `${String(w.minutes / 1440)}-day`
        : `${String(Math.round(w.minutes / 60))}-hour`;
    case "named":
      return namedWindows.get(w.label) ?? w.label;
  }
}

/** The words for a provider's verdict on a window. */
function statusWord(status: LimitStatus): string {
  return status === "rejected" ? "limit reached" : status;
}

/**
 * The full level-1 sentence of a meter, used as its `aria-valuetext` and as the visible
 * disclosure: "50 percent used, resets in 2 hours 14 minutes, today at 14:00, status
 * allowed, seen at 12:33". `observedAtMs` is the reading's time, when the caller has it.
 */
export function meterValueText(
  m: MeterView,
  nowMs: number,
  hour: HourCycle,
  observedAtMs?: number,
): string {
  const seen =
    observedAtMs === undefined
      ? []
      : [`seen at ${timeOfDay(observedAtMs, hour)}`];
  if (m.kind === "balance") {
    return [
      money(m.total_micros, m.currency),
      m.usable ? "available" : "not usable",
      ...seen,
    ].join(", ");
  }
  if (m.severity === "unknown") return "No data";
  const used = `${percentDetail(m.used_bp).replace(" %", "")} percent used`;
  if (m.resets_at === null) {
    return [
      used,
      "reset time unavailable",
      `status ${statusWord(m.status)}`,
      ...seen,
    ].join(", ");
  }
  const left = countdownSpoken(m.resets_at, nowMs);
  if (m.phase === "elapsed" || left === null) {
    return [
      "window elapsed",
      `reset ${absoluteSpoken(m.resets_at, nowMs, hour)}`,
      "waiting for your next request",
      ...seen,
    ].join(", ");
  }
  return [
    used,
    `resets in ${left}`,
    absoluteSpoken(m.resets_at, nowMs, hour),
    `status ${statusWord(m.status)}`,
    ...seen,
  ].join(", ");
}
