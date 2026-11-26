import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clock, startClock } from "./clock.svelte";

describe("clock store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("ticks once a second while the page is visible, and stops when told", () => {
    const stop = startClock();
    vi.advanceTimersByTime(3_000);
    expect(clock.now).toBe(1_003_000);
    stop();
    vi.advanceTimersByTime(3_000);
    expect(clock.now).toBe(1_003_000);
  });

  it("does not tick while the page is hidden and catches up when it shows", () => {
    let hidden = true;
    vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
    const stop = startClock();
    const before = clock.now;
    vi.advanceTimersByTime(5_000);
    expect(clock.now).toBe(before);
    hidden = false;
    document.dispatchEvent(new Event("visibilitychange"));
    expect(clock.now).toBe(1_005_000);
    stop();
  });

  it("ignores a visibility change to hidden", () => {
    vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    const stop = startClock();
    const before = clock.now;
    vi.advanceTimersByTime(2_000);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(clock.now).toBe(before);
    stop();
  });
});
