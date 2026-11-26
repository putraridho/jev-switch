import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { announce } from "../../stores/announce.svelte";
import LiveRegion from "./LiveRegion.svelte";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("LiveRegion", () => {
  it("is one polite, atomic region that shows what was announced", async () => {
    vi.useFakeTimers();
    const view = render(LiveRegion);
    const region = view.container.querySelector("[aria-live]");
    if (region === null) throw new Error("no live region");
    expect(region.getAttribute("aria-live")).toBe("polite");
    expect(region.getAttribute("aria-atomic")).toBe("true");
    expect(region.textContent.trim()).toBe("");
    announce.say("Saved");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(region.textContent.trim()).toBe("Saved");
  });
});
