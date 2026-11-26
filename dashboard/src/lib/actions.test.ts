import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { checkClaudeNow, copyText, refreshAll, retryProvider } from "./actions";
import { fixtures } from "./ipc/fixtures";
import { createMockClient } from "./ipc/mock";
import { announce, initStores } from "./stores";

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  vi.advanceTimersByTime(5_000);
  vi.useRealTimers();
});

describe("actions", () => {
  it("copies and says Copied, or says it could not", async () => {
    const write = vi.fn<(text: string) => Promise<void>>().mockResolvedValue();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: write },
      configurable: true,
    });
    expect(await copyText("codex login")).toBe(true);
    expect(write).toHaveBeenCalledWith("codex login");
    write.mockRejectedValue(new Error("no"));
    expect(await copyText("x")).toBe(false);
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toBe("Copied. Couldn't copy");
  });

  it("retries one provider and says so, and says when it could not", async () => {
    const client = createMockClient(structuredClone(fixtures.known));
    await initStores(client);
    await retryProvider("codex", "Codex");
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toBe("Checking Codex again");
    const failing = structuredClone(fixtures.known);
    failing.fail = { refresh: { code: "busy" } };
    await initStores(createMockClient(failing));
    await retryProvider("claude", "Claude");
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toBe("Couldn't start the check for Claude");
  });

  it("runs the Claude live check and says what happened", async () => {
    await initStores(createMockClient(structuredClone(fixtures.known)));
    await checkClaudeNow();
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toBe("Checking Claude now");
    const failing = structuredClone(fixtures.known);
    failing.fail = { refresh: { code: "busy" } };
    const client = createMockClient(failing);
    await initStores(client);
    await checkClaudeNow();
    await refreshAll();
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toContain("Couldn't start the check");
    expect(announce.current).toContain("Refresh failed");
  });
});
