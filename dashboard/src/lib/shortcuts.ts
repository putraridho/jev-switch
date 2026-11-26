// The keyboard (DESIGN 9.10): `R` refresh, `,` settings, `?` the list of keys (Settings,
// Behaviour), `1` to `6` open a provider in the fixed order, `Esc` back, Ctrl + plus, minus
// and zero for zoom. Single-key shortcuts act only when no text field has focus and can be
// switched off (WCAG 2.1.4); `Esc` and the Ctrl combinations always work.

import type { ProviderKey } from "./ipc/bindings/ProviderKey";

/** What a key press can do; the app wires the real ones, a test wires fakes. */
export interface KeyEnv {
  shortcutsOn: () => boolean;
  hidden: () => readonly ProviderKey[];
  order: () => readonly ProviderKey[];
  closeTopmost: () => boolean;
  canGoBack: () => boolean;
  back: () => void;
  refresh: () => void;
  openSettings: (target?: string) => void;
  openProvider: (provider: ProviderKey) => void;
  zoom: (step: "in" | "out" | "reset") => void;
}

/** The id of the Behaviour heading `?` takes the user to. */
export const behaviourId = "behaviour";

function isTextField(target: EventTarget | null): boolean {
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "submit", "reset"].includes(
      target.type,
    );
  }
  return target instanceof HTMLElement && target.isContentEditable;
}

function zoomStep(event: KeyboardEvent): "in" | "out" | "reset" | null {
  if (!event.ctrlKey || event.altKey) return null;
  if (event.key === "+" || event.key === "=") return "in";
  if (event.key === "-" || event.key === "_") return "out";
  if (event.key === "0") return "reset";
  return null;
}

/** Handles one key press; `true` when it did something (the caller then stops the event). */
export function handleKey(event: KeyboardEvent, env: KeyEnv): boolean {
  if (event.defaultPrevented) return false;
  if (event.key === "Escape") {
    if (env.closeTopmost()) return true;
    if (!env.canGoBack()) return false;
    env.back();
    return true;
  }
  const zoom = zoomStep(event);
  if (zoom !== null) {
    env.zoom(zoom);
    return true;
  }
  if (event.ctrlKey || event.metaKey || event.altKey) return false;
  if (!env.shortcutsOn() || isTextField(event.target)) return false;
  if (event.key === "r" || event.key === "R") {
    env.refresh();
    return true;
  }
  if (event.key === ",") {
    env.openSettings();
    return true;
  }
  if (event.key === "?") {
    env.openSettings(behaviourId);
    return true;
  }
  const digit = Number(event.key);
  if (Number.isInteger(digit) && digit >= 1 && digit <= env.order().length) {
    const provider = env.order()[digit - 1];
    if (provider === undefined || env.hidden().includes(provider)) return false;
    env.openProvider(provider);
    return true;
  }
  return false;
}

/** The next zoom value for a step: 100 to 200 percent in steps of 25. */
export function zoomNext(
  current: number,
  step: "in" | "out" | "reset",
): number {
  if (step === "reset") return 100;
  const next = current + (step === "in" ? 25 : -25);
  return Math.min(200, Math.max(100, next));
}
