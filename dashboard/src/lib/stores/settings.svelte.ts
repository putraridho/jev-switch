// The settings read model (docs/dashboard/DESIGN.md 10). The shape is final; T07 writes the
// rune-based body.

import type { IpcClient } from "../ipc/client";
import type { Outcome } from "../ipc/bindings/Outcome";
import type { Settings } from "../ipc/bindings/Settings";
import type { SettingsPatch } from "../ipc/bindings/SettingsPatch";

const initial: Settings = {
  version: 1,
  warn_bp: 8000,
  critical_bp: 9500,
  notifications: true,
  notify_warn: true,
  notify_critical: true,
  theme: "system",
  hour_cycle: "auto",
  zoom_pct: 100,
  close_to_tray: true,
  autostart: true,
  shortcuts: true,
  claude_live_check: false,
  first_run_seen: false,
  hidden_providers: [],
};

let value = $state<Settings>(initial);
let loaded = $state(false);
let client: IpcClient | null = null;

/** Load settings from the same client used by the usage store. */
export async function initSettings(ipc: IpcClient): Promise<void> {
  client = ipc;
  const result = await ipc.getSettings();
  if (result.status === "ok") {
    value = result.value;
    loaded = true;
  }
}

export const settings = {
  /** True once the real settings have been read (the defaults are not the user's). */
  get loaded(): boolean {
    return loaded;
  },
  get value(): Settings {
    return value;
  },
  async update(patch: SettingsPatch): Promise<Outcome<Settings>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.updateSettings(patch);
    if (result.status === "ok") value = result.value;
    return result;
  },
};
