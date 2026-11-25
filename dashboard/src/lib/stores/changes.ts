// What a new snapshot changes that a screen-reader user should hear (DESIGN 7.6, 9.10): a
// meter that changes level (ok, warn, critical) and a provider whose state changes. A
// number that moves without changing level is never announced.

import { levelChange, stateChange } from "../copy";
import type { HourCycle } from "../ipc/bindings/HourCycle";
import type { MeterView } from "../ipc/bindings/MeterView";
import type { ProviderView } from "../ipc/bindings/ProviderView";
import type { Severity } from "../ipc/bindings/Severity";
import type { Snapshot } from "../ipc/bindings/Snapshot";
import type { UsageView } from "../ipc/bindings/UsageView";

function metersOf(usage: UsageView): readonly MeterView[] {
  return usage.state === "known" || usage.state === "stale"
    ? usage.reading.meters
    : [];
}

function level(severity: Severity): boolean {
  return severity !== "unknown";
}

function levelChanges(
  before: ProviderView,
  after: ProviderView,
  nowMs: number,
  hour: HourCycle,
): string[] {
  const sentences: string[] = [];
  for (const meter of metersOf(after.usage)) {
    if (meter.kind !== "window" || meter.phase === "elapsed") continue;
    const earlier = metersOf(before.usage).find((m) => m.id === meter.id);
    if (earlier?.kind !== "window") continue;
    if (
      earlier.severity !== meter.severity &&
      level(earlier.severity) &&
      level(meter.severity)
    ) {
      sentences.push(levelChange(after.provider, meter, nowMs, hour));
    }
  }
  return sentences;
}

/** The sentences to announce for `next` after `previous`; none for the first snapshot. */
export function snapshotChanges(
  previous: Snapshot | null,
  next: Snapshot,
  nowMs: number,
  hour: HourCycle,
): string[] {
  if (previous === null) return [];
  const sentences: string[] = [];
  for (const after of next.providers) {
    if (!after.enabled) continue;
    const before = previous.providers.find(
      (item) => item.provider === after.provider,
    );
    if (before?.enabled !== true) continue;
    const state = stateChange(after.provider, before.usage, after.usage);
    if (state !== null) sentences.push(state);
    sentences.push(...levelChanges(before, after, nowMs, hour));
  }
  return sentences;
}
