// What the user's own actions do, shared by the buttons and the shortcuts, so one action
// has one announcement ("Updated 12:33", DESIGN 9.10).

import { timeOfDay } from "./format";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import { announce } from "./stores/announce.svelte";
import { settings } from "./stores/settings.svelte";
import { usage } from "./stores/usage.svelte";

/** Refreshes every provider and says how it went. */
export async function refreshAll(): Promise<void> {
  const result = await usage.refresh();
  announce.say(
    result.status === "ok"
      ? `Updated ${timeOfDay(Date.now(), settings.value.hour_cycle)}`
      : "Refresh failed",
  );
}

/** Copies `text` to the clipboard and says so (or says it could not). */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    announce.say("Copied");
    return true;
  } catch {
    announce.say("Couldn't copy");
    return false;
  }
}

/** Asks one provider to check again now, and says what happened. */
export async function retryProvider(
  provider: ProviderKey,
  name: string,
): Promise<void> {
  const result = await usage.refresh({ kind: "provider", provider });
  announce.say(
    result.status === "ok"
      ? `Checking ${name} again`
      : `Couldn't start the check for ${name}`,
  );
}

/** Runs the one small Claude request of the live check, and says what happened. */
export async function checkClaudeNow(): Promise<void> {
  const result = await usage.refresh({
    kind: "live_check",
    provider: "claude",
  });
  announce.say(
    result.status === "ok" ? "Checking Claude now" : "Couldn't start the check",
  );
}
