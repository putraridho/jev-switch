// Which `IpcClient` the app runs on. Inside Tauri (its IPC bridge is present) it is always
// the real one. Outside Tauri, in a plain browser (`pnpm dev`, the screen tests), there is no
// backend to ask, so the app runs on a named mock script from the URL fragment
// (`#fixture=mixed`, default `pending`). The mock never makes a request of any kind.

import { fixtureNames, fixtures, type FixtureName } from "./fixtures";
import type { IpcClient } from "./client";
import { createMockClient } from "./mock";
import { createTauriClient } from "./tauri";

/** The fixture the fragment names, or `pending`. */
export function fixtureFromHash(hash: string): FixtureName {
  const wanted = new URLSearchParams(hash.replace(/^#/, "")).get("fixture");
  return fixtureNames.find((name) => name === wanted) ?? "pending";
}

/** Whether the page is hosted by Tauri, whose bridge defines this global. */
function insideTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The client for this page. */
export function createClient(): IpcClient {
  return insideTauri()
    ? createTauriClient()
    : createMockClient(fixtures[fixtureFromHash(window.location.hash)]);
}
