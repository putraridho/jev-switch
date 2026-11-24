// Every user-facing sentence for Unavailable, Setup and StaleCause, and the commands to
// copy (docs/dashboard/DESIGN.md 9.9). The signatures are final; T07 writes the bodies.

import type { HourCycle } from "./ipc/bindings/HourCycle";
import type { MeterView } from "./ipc/bindings/MeterView";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import type { Setup } from "./ipc/bindings/Setup";
import type { Unavailable } from "./ipc/bindings/Unavailable";

/** What to tell the user about a failure, and the command to copy when one helps. */
export function unavailableCopy(
  _reason: Unavailable,
  _provider: ProviderKey,
): { text: string; command?: string } {
  return { text: "" };
}

/** What the user has to do first, and the command to copy when one helps. */
export function setupCopy(
  _setup: Setup,
  _provider: ProviderKey,
): { text: string; command?: string } {
  return { text: "" };
}

/** The honest "no source yet" sentence: no number, no plan. */
export function unsupportedCopy(_provider: ProviderKey): string {
  return "";
}

/** The display name of each provider. */
export const providerName: Record<ProviderKey, string> = {
  claude: "Claude",
  codex: "Codex",
  deepseek: "DeepSeek",
  gemini: "Gemini",
  antigravity: "Antigravity",
  grok: "Grok",
};

/** "max_5x" becomes "Max 5x", "free" becomes "Free". */
export function planName(_label: string): string {
  return "";
}

/** The live-region sentence for a level change: the same words as the notification. */
export function levelChange(
  _provider: ProviderKey,
  _meter: MeterView,
  _nowMs: number,
  _hour: HourCycle,
): string {
  return "";
}
