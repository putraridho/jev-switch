// How the overview orders and splits the providers (DESIGN 9.3): cards by severity
// (critical, warn, ok, unknown) then by the fixed provider order; providers without a usage
// source become one quiet block at the bottom; hidden providers have no card.

import { providerOrder } from "./copy";
import type { ProviderView } from "./ipc/bindings/ProviderView";
import type { Severity } from "./ipc/bindings/Severity";
import type { Snapshot } from "./ipc/bindings/Snapshot";

const rank: Record<Severity, number> = {
  critical: 0,
  warn: 1,
  ok: 2,
  unknown: 3,
};

/** The worst level among a provider's meters; `unknown` when it has no reading. */
export function worstSeverity(provider: ProviderView): Severity {
  const usage = provider.usage;
  if (usage.state !== "known" && usage.state !== "stale") return "unknown";
  let worst: Severity = "unknown";
  for (const meter of usage.reading.meters) {
    if (rank[meter.severity] < rank[worst]) worst = meter.severity;
  }
  return worst;
}

/** Does this card draw a waffle (now or while loading)? The rest sit under "Other providers"
 * (S31, S50). */
export function hasWaffle(provider: ProviderView): boolean {
  const usage = provider.usage;
  if (usage.state === "pending") return true;
  if (usage.state !== "known" && usage.state !== "stale") return false;
  return usage.reading.meters.some((meter) => meter.kind === "window");
}

/** The cards in display order, and the names of the providers with no usage source. */
export function arrange(snapshot: Snapshot): {
  cards: ProviderView[];
  unsupported: ProviderView[];
} {
  const shown = snapshot.providers.filter((item) => item.enabled);
  const order = (item: ProviderView): number =>
    providerOrder.indexOf(item.provider);
  return {
    cards: shown
      .filter((item) => item.usage.state !== "unsupported")
      .sort(
        (a, b) =>
          rank[worstSeverity(a)] - rank[worstSeverity(b)] ||
          order(a) - order(b),
      ),
    unsupported: shown
      .filter((item) => item.usage.state === "unsupported")
      .sort((a, b) => order(a) - order(b)),
  };
}
