// Wires the stores to an `IpcClient` (docs/dashboard/DESIGN.md 7.3). T07 writes the body.

import type { IpcClient } from "../ipc/client";
import { initRouter } from "../router.svelte";
import { initQueries } from "./queries";
import { initSettings } from "./settings.svelte";
import { initUsage } from "./usage.svelte";

export { announce } from "./announce.svelte";
export { clock } from "./clock.svelte";
export { settings } from "./settings.svelte";
export { queries } from "./queries";
export { usage } from "./usage.svelte";

/**
 * Subscribes to `snapshot` BEFORE calling `getSnapshot`, loads the settings and the
 * navigate listener. Resolves to the function that unsubscribes everything.
 */
export async function initStores(client: IpcClient): Promise<() => void> {
  initQueries(client);
  const unsubscribeUsage = await initUsage(client);
  await initSettings(client);
  const unsubscribeRouter = await initRouter(client);
  return () => {
    unsubscribeUsage();
    unsubscribeRouter();
  };
}
