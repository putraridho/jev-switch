// Wires the stores to an `IpcClient` (docs/dashboard/DESIGN.md 7.3).

import type { IpcClient } from "../ipc/client";
import { initRouter } from "../router.svelte";
import { initChat } from "./chat.svelte";
import { initQueries } from "./queries";
import { initSettings } from "./settings.svelte";
import { initUsage } from "./usage.svelte";

export { announce } from "./announce.svelte";
export { chat } from "./chat.svelte";
export { clock } from "./clock.svelte";
export { settings } from "./settings.svelte";
export { queries } from "./queries";
export { usage } from "./usage.svelte";

/**
 * Subscribes to `snapshot` and `chat_event` BEFORE fetching initial state, loads
 * settings and the navigate listener. Resolves to the function that unsubscribes all.
 */
export async function initStores(client: IpcClient): Promise<() => void> {
  initQueries(client);
  const unsubscribeUsage = await initUsage(client);
  const unsubscribeChat = await initChat(client);
  await initSettings(client);
  const unsubscribeRouter = await initRouter(client);
  return () => {
    unsubscribeUsage();
    unsubscribeChat();
    unsubscribeRouter();
  };
}
