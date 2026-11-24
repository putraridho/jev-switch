// The settings read model (docs/dashboard/DESIGN.md 10). The shape is final; T07 writes the
// rune-based body.

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

/** The current settings and the one way to change them. */
export const settings: {
  readonly value: Settings;
  update(p: SettingsPatch): Promise<Outcome<Settings>>;
} = {
  value: initial,
  update: () =>
    Promise.resolve({ status: "error", error: { code: "internal" } }),
};
