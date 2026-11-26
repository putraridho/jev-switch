// Every user-facing sentence for Unavailable, Setup and StaleCause, and the commands to
// copy (docs/dashboard/DESIGN.md 9.9). Tone: plain, specific, calm; sentence case; the fact,
// then the next step. No error codes, no "Something went wrong".

import { absolute, age, percent, windowName } from "./format";
import type { HourCycle } from "./ipc/bindings/HourCycle";
import type { LimitStatus } from "./ipc/bindings/LimitStatus";
import type { MeterView } from "./ipc/bindings/MeterView";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import type { Setup } from "./ipc/bindings/Setup";
import type { Unavailable } from "./ipc/bindings/Unavailable";
import type { UsageView } from "./ipc/bindings/UsageView";

/** The display name of each provider. */
export const providerName: Record<ProviderKey, string> = {
  claude: "Claude",
  codex: "Codex",
  deepseek: "DeepSeek",
  gemini: "Gemini",
  antigravity: "Antigravity",
  grok: "Grok",
};

/** What each provider's row says in Settings (S53). */
export const providerHint: Record<ProviderKey, string> = {
  claude: "5-hour and 7-day windows.",
  codex: "5-hour and weekly windows.",
  deepseek: "Balance.",
  gemini: "No source Flashboard can read yet.",
  antigravity: "Gemini and Claude/GPT limits.",
  grok: "No source Flashboard can read yet.",
};

/** The fixed order of `ProviderKey::ALL`: the digits 1 to 6 follow it. */
export const providerOrder: readonly ProviderKey[] = [
  "claude",
  "codex",
  "deepseek",
  "gemini",
  "antigravity",
  "grok",
];

/** The login commands: the same strings as `engine_bridge::login_command`. */
const loginCommand: Record<ProviderKey, string | undefined> = {
  claude: "claude auth login",
  codex: "codex login",
  deepseek: undefined,
  gemini: undefined,
  antigravity: undefined,
  grok: undefined,
};

/** What to tell the user about a failure, and the command to copy when one helps. */
export function unavailableCopy(
  reason: Unavailable,
  provider: ProviderKey,
): { text: string; command?: string } {
  const name = providerName[provider];
  switch (reason) {
    case "not_installed":
      return { text: `${name} isn't installed on this computer.` };
    case "not_logged_in": {
      // `agy` answers the same blank way when signed out and before any use.
      if (provider === "antigravity") {
        return {
          text: "No Antigravity usage to show. Use agy once, and check you're signed in.",
        };
      }
      const command = loginCommand[provider];
      return command === undefined
        ? { text: `Not signed in to ${name}.` }
        : { text: `Not signed in to ${name}.`, command };
    }
    case "offline":
      return { text: `Can't reach ${name}. Check your connection.` };
    case "timeout":
      return { text: `${name} took too long to answer. It will try again.` };
    case "rate_limited":
      return {
        text: `${name} asked us to slow down. It will try again later.`,
      };
    case "unauthorized":
      return {
        text: `${name} didn't accept the saved credentials.`,
        ...(loginCommand[provider] === undefined
          ? {}
          : { command: loginCommand[provider] }),
      };
    case "server_error":
      return { text: `${name} had a problem on its side. It will try again.` };
    case "protocol":
      return { text: `${name} sent an answer this app doesn't understand.` };
    case "io":
      return { text: `Couldn't read ${name}'s data on this computer.` };
    case "internal":
      return { text: `${name} could not be checked. It will try again.` };
  }
}

/** What the user has to do first, and the command to copy when one helps. */
export function setupCopy(
  setup: Setup,
  provider: ProviderKey,
): { text: string; command?: string } {
  const name = providerName[provider];
  switch (setup) {
    case "use_gateway":
      return {
        text: `No ${name} usage seen yet. Run jev-claude once and it appears here.`,
        command: "jev-claude",
      };
    case "add_deepseek_key":
      return {
        text: `Add a ${name} key to see your balance.`,
        command: "DEEPSEEK_API_KEY",
      };
  }
}

/** The honest "no source yet" sentence: no number, no plan. */
export function unsupportedCopy(provider: ProviderKey): string {
  return `${providerName[provider]} has no usage source yet.`;
}

/** "max_5x" becomes "Max 5x", "free" becomes "Free". */
export function planName(label: string): string {
  const words: string[] = label.split("_").filter((word) => word !== "");
  const first = words[0];
  if (first === undefined) return "";
  return [
    first.charAt(0).toUpperCase() + first.slice(1),
    ...words.slice(1),
  ].join(" ");
}

/** The words for a provider's verdict on a window. */
export const statusName: Record<LimitStatus, string> = {
  allowed: "Allowed",
  warning: "Warning",
  rejected: "Limit reached",
  unknown: "Unknown",
};

/**
 * The live-region sentence for a level change: the same words as the notification
 * (`text::notification`): "Claude 5-hour at 82%. Resets today 14:00."
 */
export function levelChange(
  provider: ProviderKey,
  meter: MeterView,
  nowMs: number,
  hour: HourCycle,
): string {
  if (meter.kind !== "window") {
    return `${providerName[provider]} balance is ${meter.usable ? "available" : "not usable"}.`;
  }
  const head =
    meter.severity === "ok"
      ? `${providerName[provider]} ${windowName(meter.window)} is back to normal at ${percent(meter.used_bp)}.`
      : `${providerName[provider]} ${windowName(meter.window)} at ${percent(meter.used_bp)}.`;
  return meter.resets_at === null
    ? head
    : `${head} Resets ${absolute(meter.resets_at, nowMs, hour)}.`;
}

/** The live-region sentence when a provider's state changes, or `null` when it does not. */
export function stateChange(
  provider: ProviderKey,
  before: UsageView | undefined,
  after: UsageView,
): string | null {
  if (before === undefined) return null;
  const name = providerName[provider];
  if (before.state === "stale" && after.state === "stale") {
    if (before.cause.kind === after.cause.kind) return null;
    return after.cause.kind === "failing"
      ? `Can't reach ${name}. Showing the last reading.`
      : `${name} is quiet. Showing the last reading.`;
  }
  if (before.state === after.state) return null;
  switch (after.state) {
    case "unavailable":
      return unavailableCopy(after.reason, provider).text;
    case "stale":
      return after.cause.kind === "failing"
        ? `Can't reach ${name}. Showing the last reading.`
        : `${name} is quiet. Showing the last reading.`;
    case "known":
      return before.state === "pending" ? null : `${name} is back.`;
    case "not_configured":
      return setupCopy(after.setup, provider).text;
    case "pending":
    case "unsupported":
      return null;
  }
}

/** The one quiet line of a stale card: when it was last seen, and why it is old. */
export function staleNote(
  provider: ProviderKey,
  usage: UsageView,
  nowMs: number,
  hour: HourCycle,
): string | null {
  if (usage.state !== "stale") return null;
  const seen = age(usage.reading.observed_at, nowMs, hour);
  return usage.cause.kind === "failing"
    ? `Can't reach ${providerName[provider]} · last seen ${seen}`
    : `Last seen ${seen} · updates when ${providerName[provider]} is used`;
}

/** The sentence for the providers that have no usage source, as one quiet line. */
export function unsupportedLine(providers: readonly ProviderKey[]): string {
  const names = providers.map((provider) => providerName[provider]);
  const last = names.pop();
  if (last === undefined) return "";
  const list = names.length === 0 ? last : `${names.join(", ")} and ${last}`;
  return `${list} ${providers.length === 1 ? "has" : "have"} no usage source yet.`;
}

/** Where a provider's numbers come from, in one quiet line (DESIGN 9.4). */
export function sourceLine(provider: ProviderKey): string {
  switch (provider) {
    case "claude":
      return "Source: gateway log (updates when you use jev-claude)";
    case "codex":
      return "Source: the Codex app-server (asked every few minutes)";
    case "deepseek":
      return "Source: DeepSeek balance (asked every few minutes)";
    case "antigravity":
      return "Source: agy /usage (asked every 15 minutes)";
    case "gemini":
    case "grok":
      return "Source: none yet";
  }
}

/** Can a failed check be tried again by pressing a button? */
export function retryable(reason: Unavailable): boolean {
  return !(
    reason === "not_installed" ||
    reason === "not_logged_in" ||
    reason === "unauthorized"
  );
}

/** One provider's line on the first-run screen: a glyph (decoration) and the words. */
export function firstRunLine(
  provider: ProviderKey,
  usage: UsageView,
): { glyph: string; text: string } {
  const name = providerName[provider];
  switch (usage.state) {
    case "known":
    case "stale":
      return { glyph: "✓", text: `${name} · found` };
    case "pending":
      return { glyph: "…", text: `${name} · checking` };
    case "not_configured":
      return {
        glyph: "○",
        text:
          usage.setup === "add_deepseek_key"
            ? `${name} · add a key`
            : `${name} · run jev-claude once`,
      };
    case "unavailable":
      return {
        glyph: "▲",
        text:
          usage.reason === "not_installed"
            ? `${name} · not installed`
            : usage.reason === "not_logged_in" ||
                usage.reason === "unauthorized"
              ? `${name} · not signed in`
              : `${name} · can't reach it yet`,
      };
    case "unsupported":
      return { glyph: "○", text: `${name} · no usage source yet` };
  }
}
