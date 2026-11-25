// The two threshold fields of Settings (DESIGN 9.6): what is allowed, and the sentence that
// says so next to the field. The core validates again; this keeps the explanation local.

/** A checked value in basis points, or the sentence to show. */
export type ThresholdCheck =
  { ok: true; bp: number } | { ok: false; message: string };

const MIN = 50;
const MAX = 100;

function inRange(percent: number): boolean {
  return Number.isInteger(percent) && percent >= MIN && percent <= MAX;
}

/** The warning level: 50 to 100, below the critical level. */
export function checkWarn(percent: number, criticalBp: number): ThresholdCheck {
  if (!inRange(percent)) {
    return {
      ok: false,
      message: `Enter a whole number from ${String(MIN)} to ${String(MAX)}`,
    };
  }
  if (percent * 100 >= criticalBp) {
    return {
      ok: false,
      message: `Warn must be below critical (${String(Math.round(criticalBp / 100))})`,
    };
  }
  return { ok: true, bp: percent * 100 };
}

/** The critical level: 50 to 100, above the warning level. */
export function checkCritical(percent: number, warnBp: number): ThresholdCheck {
  if (!inRange(percent)) {
    return {
      ok: false,
      message: `Enter a whole number from ${String(MIN)} to ${String(MAX)}`,
    };
  }
  if (percent * 100 <= warnBp) {
    return {
      ok: false,
      message: `Critical must be above warn (${String(Math.round(warnBp / 100))})`,
    };
  }
  return { ok: true, bp: percent * 100 };
}
