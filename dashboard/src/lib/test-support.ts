// Small stand-ins for browser APIs jsdom lacks, shared by the component tests.

import { vi } from "vitest";

/** `bind:clientWidth` needs a `ResizeObserver`; this one records what it was asked to watch. */
export function stubResizeObserver(): void {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      readonly watched: Element[] = [];
      observe(target: Element): void {
        this.watched.push(target);
      }
      unobserve(): void {
        this.watched.length = 0;
      }
      disconnect(): void {
        this.watched.length = 0;
      }
    },
  );
}
