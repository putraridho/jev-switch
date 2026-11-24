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
function initialRoute(): Route {
  if (typeof window === "undefined") return { name: "overview" };
  const params = new URLSearchParams(window.location.hash.slice(1));
  if (params.get("view") === "settings") return { name: "settings" };
  if (params.get("view") === "provider") {
    const provider = providers.find((key) => key === params.get("provider"));
    if (provider !== undefined) return { name: "provider", provider };
  }
  return { name: "overview" };
}
let route = $state<Route>(initialRoute());
const history: Route[] = [];
export const router = {
  get route(): Route {
    return route;
  },
  go(next: Route): void {
    history.push(route);
    route = next;
  },
  back(): void {
    route = history.pop() ?? { name: "overview" };
  },
};
export function initRouter(client: IpcClient): Promise<() => void> {
  return client.onNavigate((next) => {
    router.go(next);
  });
}
