// The usage read model (docs/dashboard/DESIGN.md 10). The shape is final; T07 writes the
// rune-based body.

import type { Outcome } from "../ipc/bindings/Outcome";
import type { RefreshTarget } from "../ipc/bindings/RefreshTarget";
import type { Snapshot } from "../ipc/bindings/Snapshot";

/** The latest snapshot and the manual refresh. */
export const usage: {
  readonly snapshot: Snapshot | null;
  readonly refreshing: boolean;
  refresh(t?: RefreshTarget): Promise<Outcome<null>>;
} = {
  snapshot: null,
  refreshing: false,
  refresh: () =>
    Promise.resolve({ status: "error", error: { code: "internal" } }),
};
