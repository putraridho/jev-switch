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

/** A mock client that answers from `script`. */
export function createMockClient(script: MockScript): MockIpcClient {
  const calls: MockCall[] = [];
  const snapshots = new Set<(snapshot: Snapshot) => void>();
  const routes = new Set<(route: Route) => void>();
  let currentSnapshot = script.snapshot;
  let currentSettings = { ...script.settings };
  return {
    getSnapshot: () => {
      calls.push({ command: "getSnapshot" });
      return Promise.resolve({ status: "ok", value: currentSnapshot });
    },
    refresh: () => {
      calls.push({ command: "refresh" });
      const error = script.fail.refresh;
      return Promise.resolve(
        error ? { status: "error", error } : { status: "ok", value: null },
      );
    },
    getHistory: () => {
      calls.push({ command: "getHistory" });
      const error = script.fail.getHistory;
      return Promise.resolve(
        error
          ? { status: "error", error }
          : { status: "ok", value: script.history },
      );
    },
    getSettings: () => {
      calls.push({ command: "getSettings" });
      return Promise.resolve({ status: "ok", value: currentSettings });
    },
    updateSettings: (patch) => {
      calls.push({ command: "updateSettings" });
      const error = script.fail.updateSettings;
      if (error) return Promise.resolve({ status: "error", error });
      currentSettings = { ...currentSettings, ...patch };
      return Promise.resolve({ status: "ok", value: currentSettings });
    },
    getDiagnostics: () => {
      calls.push({ command: "getDiagnostics" });
      return Promise.resolve({ status: "ok", value: script.diagnostics });
    },
    onSnapshot: (cb) => {
      snapshots.add(cb);
      return Promise.resolve(() => {
        snapshots.delete(cb);
      });
    },
    onNavigate: (cb) => {
      routes.add(cb);
      return Promise.resolve(() => {
        routes.delete(cb);
      });
    },
    push: (snapshot) => {
      currentSnapshot = snapshot;
      snapshots.forEach((cb) => {
        cb(snapshot);
      });
    },
    navigate: (route) => {
      routes.forEach((cb) => {
        cb(route);
      });
    },
    calls,
  };
}
