// Wires the stores to an `IpcClient` (docs/dashboard/DESIGN.md 7.3). T07 writes the body.

import type { IpcClient } from "../ipc/client";

export { announce } from "./announce.svelte";
export { clock } from "./clock.svelte";
export { settings } from "./settings.svelte";
export { usage } from "./usage.svelte";

/**
 * Subscribes to `snapshot` BEFORE calling `getSnapshot` and keeps the greater
 * `generated_at`. Resolves to the function that unsubscribes.
 */
export function initStores(_client: IpcClient): Promise<() => void> {
  return Promise.resolve(() => {
    return;
  });
}
