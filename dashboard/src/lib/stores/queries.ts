// The two read-only questions a screen can ask after boot: a history series and the
// diagnostics. They go through the same `IpcClient` the stores use (DESIGN 7.3).

import type { Diagnostics } from "../ipc/bindings/Diagnostics";
import type { HistoryQuery } from "../ipc/bindings/HistoryQuery";
import type { HistorySeries } from "../ipc/bindings/HistorySeries";
import type { Outcome } from "../ipc/bindings/Outcome";
import type { IpcClient } from "../ipc/client";

let client: IpcClient | null = null;

/** Remembers the client; called once by `initStores`. */
export function initQueries(ipc: IpcClient): void {
  client = ipc;
}

const unavailable = { status: "error", error: { code: "internal" } } as const;

export const queries = {
  history(query: HistoryQuery): Promise<Outcome<HistorySeries>> {
    return client === null
      ? Promise.resolve(unavailable)
      : client.getHistory(query);
  },
  diagnostics(): Promise<Outcome<Diagnostics>> {
    return client === null
      ? Promise.resolve(unavailable)
      : client.getDiagnostics();
  },
};
