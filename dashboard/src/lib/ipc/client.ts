// The IPC seam (docs/dashboard/DESIGN.md 7.3, 10). Components and stores import ONLY this
// interface, never `@tauri-apps/api`. Every type is generated from Rust; nothing here is
// hand-typed twice. Frozen after T00.

import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistoryQuery } from "./bindings/HistoryQuery";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Outcome } from "./bindings/Outcome";
import type { RefreshTarget } from "./bindings/RefreshTarget";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { SettingsPatch } from "./bindings/SettingsPatch";
import type { Snapshot } from "./bindings/Snapshot";

/** The six commands and the two events. Every command resolves with an `Outcome`; none rejects. */
export interface IpcClient {
  getSnapshot(): Promise<Outcome<Snapshot>>;
  refresh(target: RefreshTarget): Promise<Outcome<null>>;
  getHistory(query: HistoryQuery): Promise<Outcome<HistorySeries>>;
  getSettings(): Promise<Outcome<Settings>>;
  updateSettings(patch: SettingsPatch): Promise<Outcome<Settings>>;
  getDiagnostics(): Promise<Outcome<Diagnostics>>;
  onSnapshot(cb: (s: Snapshot) => void): Promise<() => void>;
  onNavigate(cb: (r: Route) => void): Promise<() => void>;
}
