import { SvelteURLSearchParams } from "svelte/reactivity";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import type { Route } from "./ipc/bindings/Route";
import type { IpcClient } from "./ipc/client";

const providers: readonly ProviderKey[] = [
  "claude",
  "codex",
  "deepseek",
  "gemini",
  "antigravity",
  "grok",
];

/** One step back: where to go, and the control that opened the view being left. */
type Entry = { route: Route; trigger: HTMLElement | null };

/** The view a URL fragment names (DESIGN 7.3): `view`, and `provider` for a provider. */
export function routeFromHash(hash: string): Route {
  const params = new SvelteURLSearchParams(hash.replace(/^#/, ""));
  if (params.get("view") === "chat") return { name: "chat" };
  if (params.get("view") === "settings") return { name: "settings" };
  if (params.get("view") === "provider") {
    const provider = providers.find((key) => key === params.get("provider"));
    if (provider !== undefined) return { name: "provider", provider };
  }
  return { name: "overview" };
}

function initialRoute(): Route {
  return typeof window === "undefined"
    ? { name: "overview" }
    : routeFromHash(window.location.hash);
}

/** The focused control, or `null` when nothing but the page itself has focus. */
function focusedControl(): HTMLElement | null {
  if (typeof document === "undefined") return null;
  const active = document.activeElement;
  return active instanceof HTMLElement && active !== document.body
    ? active
    : null;
}

let route = $state<Route>(initialRoute());
const history: Entry[] = [];
let returnTo: HTMLElement | null = null;
let section: string | null = null;

export const router = {
  get route(): Route {
    return route;
  },
  /** Whether `back` leaves the current view. */
  get canGoBack(): boolean {
    return history.length > 0 || route.name !== "overview";
  },
  /** Opens `next`; `target` names an element id to focus once the view is shown. */
  go(next: Route, target?: string): void {
    history.push({ route, trigger: focusedControl() });
    route = next;
    returnTo = null;
    section = target ?? null;
  },
  back(): void {
    const entry = history.pop();
    route = entry?.route ?? { name: "overview" };
    returnTo = entry?.trigger ?? null;
    section = null;
  },
  /** The control to give focus back to after `back`, once; `null` when there is none. */
  takeReturnFocus(): HTMLElement | null {
    const trigger = returnTo;
    returnTo = null;
    return trigger;
  },
  /** The element id `go` asked to focus, once. */
  takeSection(): string | null {
    const target = section;
    section = null;
    return target;
  },
};

export function initRouter(client: IpcClient): Promise<() => void> {
  return client.onNavigate((next) => {
    router.go(next);
  });
}
