// The in-memory router (docs/dashboard/DESIGN.md 7.3, 10): the initial route and theme come
// from the URL fragment. The shape is final; T07 writes the rune-based body.

import type { Route } from "./ipc/bindings/Route";

const overview: Route = { name: "overview" };

/** The current screen and the two ways to change it. */
export const router: {
  readonly route: Route;
  go(r: Route): void;
  back(): void;
} = {
  route: overview,
  go: () => {
    return;
  },
  back: () => {
    return;
  },
};
