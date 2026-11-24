// The Tauri implementation of `IpcClient`: the one file that may import `@tauri-apps/api`
// (docs/dashboard/DESIGN.md 7.3). T07 writes the body; this stub answers `internal`.

import type { Outcome } from "./bindings/Outcome";
import type { IpcClient } from "./client";

function notWired<T>(): Promise<Outcome<T>> {
  return Promise.resolve({ status: "error", error: { code: "internal" } });
}

function noListener(): Promise<() => void> {
  return Promise.resolve(() => {
    return;
  });
}

/** The client the real app uses. */
export function createTauriClient(): IpcClient {
  return {
    getSnapshot: () => notWired(),
    refresh: () => notWired(),
    getHistory: () => notWired(),
    getSettings: () => notWired(),
    updateSettings: () => notWired(),
    getDiagnostics: () => notWired(),
    onSnapshot: () => noListener(),
    onNavigate: () => noListener(),
  };
}
