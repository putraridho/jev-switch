// The mock `IpcClient` for unit, component and screen tests (docs/dashboard/DESIGN.md 10).
// The types are final; T07 writes the body.

import type { CommandError } from "./bindings/CommandError";
import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { IpcClient } from "./client";

/** Everything a mock client answers with. */
export type MockScript = {
  snapshot: Snapshot;
  settings: Settings;
  history: HistorySeries;
  diagnostics: Diagnostics;
  fail: Partial<
    Record<"refresh" | "getHistory" | "updateSettings", CommandError>
  >;
};

/** One recorded command call. */
export type MockCall = {
  command:
    | "getSnapshot"
    | "refresh"
    | "getHistory"
    | "getSettings"
    | "updateSettings"
    | "getDiagnostics";
};

/** A client a test can drive: push snapshots, send routes, read the calls made. */
export interface MockIpcClient extends IpcClient {
  push(s: Snapshot): void;
  navigate(r: Route): void;
  readonly calls: readonly MockCall[];
}

function noListener(): Promise<() => void> {
  return Promise.resolve(() => {
    return;
  });
}

/** A mock client that answers from `script`. */
export function createMockClient(script: MockScript): MockIpcClient {
  const calls: MockCall[] = [];
  return {
    getSnapshot: () =>
      Promise.resolve({ status: "ok", value: script.snapshot }),
    refresh: () => Promise.resolve({ status: "ok", value: null }),
    getHistory: () => Promise.resolve({ status: "ok", value: script.history }),
    getSettings: () =>
      Promise.resolve({ status: "ok", value: script.settings }),
    updateSettings: () =>
      Promise.resolve({ status: "ok", value: script.settings }),
    getDiagnostics: () =>
      Promise.resolve({ status: "ok", value: script.diagnostics }),
    onSnapshot: () => noListener(),
    onNavigate: () => noListener(),
    push: () => {
      return;
    },
    navigate: () => {
      return;
    },
    calls,
  };
}
