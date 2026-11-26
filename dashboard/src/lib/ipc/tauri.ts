import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Outcome } from "./bindings/Outcome";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { IpcClient } from "./client";

/** The client the real app uses. */
export function createTauriClient(): IpcClient {
  return {
    getSnapshot: () =>
      invoke<Outcome<Snapshot>>("get_snapshot").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    refresh: (target) =>
      invoke<Outcome<null>>("refresh", { target }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getHistory: (query) =>
      invoke<Outcome<HistorySeries>>("get_history", { query }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getSettings: () =>
      invoke<Outcome<Settings>>("get_settings").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    updateSettings: (patch) =>
      invoke<Outcome<Settings>>("update_settings", { patch }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getDiagnostics: () =>
      invoke<Outcome<Diagnostics>>("get_diagnostics").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    onSnapshot: (cb) =>
      listen<Snapshot>("snapshot", (event) => {
        cb(event.payload);
      }),
    onNavigate: (cb) =>
      listen<Route>("navigate", (event) => {
        cb(event.payload);
      }),
  };
}
