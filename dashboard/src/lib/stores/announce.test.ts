import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { announce } from "./announce.svelte";

// The store keeps its state for the life of the module, so each test uses sentences of its
// own and flushes the merge timer before it ends.
describe("live region store", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts silent", () => {
    expect(announce.current).toBe("");
  });

  it("merges what happens within two seconds into one sentence", () => {
    announce.say("Claude 5-hour at 82%.");
    vi.advanceTimersByTime(1_000);
    announce.say("Updated 12:33");
    expect(announce.current).toBe("");
    vi.advanceTimersByTime(1_000);
    expect(announce.current).toBe("Claude 5-hour at 82%.. Updated 12:33");
  });

  it("does not repeat the same sentence within 60 seconds", () => {
    announce.say("Saved");
    vi.advanceTimersByTime(2_000);
    expect(announce.current).toBe("Saved");
    announce.say("Other");
    vi.advanceTimersByTime(2_000);
    announce.say("Saved");
    vi.advanceTimersByTime(2_000);
    expect(announce.current).toBe("Other");
    vi.advanceTimersByTime(60_000);
    announce.say("Saved");
    vi.advanceTimersByTime(2_000);
    expect(announce.current).toBe("Saved");
  });
});
