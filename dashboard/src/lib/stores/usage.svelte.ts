import type { IpcClient } from "../ipc/client";
import type { Outcome } from "../ipc/bindings/Outcome";
import type { RefreshTarget } from "../ipc/bindings/RefreshTarget";
import type { Snapshot } from "../ipc/bindings/Snapshot";
import { announce } from "./announce.svelte";
import { snapshotChanges } from "./changes";
import { settings } from "./settings.svelte";

let snapshot = $state<Snapshot | null>(null);
let refreshing = $state(false);
let client: IpcClient | null = null;
let pending = 0;

/** Subscribe before reading the initial snapshot so no newer event is lost. */
export async function initUsage(ipc: IpcClient): Promise<() => void> {
  client = ipc;
  snapshot = null;
  let active = true;
  const accept = (next: Snapshot) => {
    if (
      active &&
      (snapshot === null || next.generated_at > snapshot.generated_at)
    ) {
      // A level or state change is announced once, from the difference of two snapshots.
      for (const sentence of snapshotChanges(
        snapshot,
        next,
        Date.now(),
        settings.value.hour_cycle,
      )) {
        announce.say(sentence);
      }
      snapshot = next;
    }
  };
  const unsubscribe = await ipc.onSnapshot(accept);
  const result = await ipc.getSnapshot();
  if (result.status === "ok") accept(result.value);
  return () => {
    active = false;
    unsubscribe();
  };
}

export const usage = {
  get snapshot(): Snapshot | null {
    return snapshot;
  },
  get refreshing(): boolean {
    return refreshing;
  },
  async refresh(
    target: RefreshTarget = { kind: "all" },
  ): Promise<Outcome<null>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    pending += 1;
    refreshing = true;
    const result = await client.refresh(target);
    pending -= 1;
    refreshing = pending > 0;
    return result;
  },
};
