// A provider's plan usage as the chat shows it: the one window most likely to stop the next
// turn (the fullest), and the share of it a turn used.
import type { MeterView } from "../ipc/bindings/MeterView";
import type { ProviderKey } from "../ipc/bindings/ProviderKey";
import type { Snapshot } from "../ipc/bindings/Snapshot";

type WindowMeter = Extract<MeterView, { kind: "window" }>;

export interface PlanUsage {
  meter: WindowMeter;
  plan: string | null;
  /** The last good reading after a failed one. */
  stale: boolean;
}

/** The fullest window of a provider with a reading; `null` when there is none. */
export function planUsage(
  snapshot: Snapshot | null,
  provider: ProviderKey,
): PlanUsage | null {
  const view = snapshot?.providers.find((p) => p.provider === provider);
  if (view === undefined) return null;
  const u = view.usage;
  if (u.state !== "known" && u.state !== "stale") return null;
  const windows: WindowMeter[] = u.reading.meters.flatMap((m) =>
    m.kind === "window" ? [m] : [],
  );
  const fullest = windows.reduce<WindowMeter | null>(
    (best, m) => (best === null || m.used_bp > best.used_bp ? m : best),
    null,
  );
  if (fullest === null) return null;
  return { meter: fullest, plan: u.reading.plan, stale: u.state === "stale" };
}

/** Each provider's fullest-window usage in basis points, for remembering at send time. */
export function usageByProvider(
  snapshot: Snapshot | null,
): Partial<Record<ProviderKey, number>> {
  const out: Partial<Record<ProviderKey, number>> = {};
  for (const view of snapshot?.providers ?? []) {
    const usage = planUsage(snapshot, view.provider);
    if (usage !== null) out[view.provider] = usage.meter.used_bp;
  }
  return out;
}

/** Basis points a turn used, or `null` when nothing measurable changed (or it can't tell). */
export function turnSpend(
  beforeBp: number | undefined,
  afterBp: number | undefined,
): number | null {
  if (beforeBp === undefined || afterBp === undefined) return null;
  const spent = afterBp - beforeBp;
  return spent > 0 ? spent : null;
}
