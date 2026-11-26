import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fixtures, type FixtureName } from "../ipc/fixtures";
import { createMockClient, type MockScript } from "../ipc/mock";
import { router } from "../router.svelte";
import { announce, initStores, settings, usage } from "./index";

function script(name: FixtureName): MockScript {
  return structuredClone(fixtures[name]);
}

const base = fixtures.known.snapshot.generated_at;

// The stores keep their state for the life of the module. `initStores` resets the snapshot
// and loads the settings again, so every test starts by wiring a fresh client; the first
// test runs before any client is wired.
describe("stores", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.advanceTimersByTime(5_000);
    vi.useRealTimers();
  });

  it("refuses to refresh or save before it is wired", async () => {
    expect((await usage.refresh()).status).toBe("error");
    expect((await settings.update({ zoom_pct: 125 })).status).toBe("error");
    expect(settings.loaded).toBe(false);
  });

  it("knows when the user's own settings have been read", async () => {
    await initStores(createMockClient(script("known")));
    expect(settings.loaded).toBe(true);
  });

  it("subscribes before it reads and shows the first snapshot", async () => {
    const client = createMockClient(script("known"));
    const stop = await initStores(client);
    expect(usage.snapshot?.providers.length).toBe(6);
    expect(client.calls.map((c) => c.command)).toContain("getSnapshot");
    stop();
  });

  it("keeps the newest snapshot and ignores an older one", async () => {
    const client = createMockClient(script("known"));
    await initStores(client);
    client.push({ ...fixtures.warn.snapshot, generated_at: base + 1 });
    expect(usage.snapshot?.generated_at).toBe(base + 1);
    client.push({ ...fixtures.known.snapshot, generated_at: 1 });
    expect(usage.snapshot?.generated_at).toBe(base + 1);
  });

  it("announces a level change once, from consecutive snapshots", async () => {
    const client = createMockClient(script("known"));
    await initStores(client);
    client.push({ ...fixtures.warn.snapshot, generated_at: base + 2 });
    vi.advanceTimersByTime(2_000);
    expect(announce.current).toMatch(/^Claude 5-hour at 86%/);
  });

  it("stops listening once unsubscribed", async () => {
    const client = createMockClient(script("known"));
    const stop = await initStores(client);
    stop();
    client.push({ ...fixtures.warn.snapshot, generated_at: base + 5 });
    expect(usage.snapshot?.generated_at).toBe(base);
  });

  it("tracks a refresh in flight and reports its outcome", async () => {
    const client = createMockClient(script("known"));
    await initStores(client);
    const pending = usage.refresh();
    expect(usage.refreshing).toBe(true);
    const result = await pending;
    expect(result.status).toBe("ok");
    expect(usage.refreshing).toBe(false);
  });

  it("loads the settings and applies an update", async () => {
    const client = createMockClient(script("known"));
    await initStores(client);
    expect(settings.value.warn_bp).toBe(8000);
    const result = await settings.update({ zoom_pct: 125 });
    expect(result.status).toBe("ok");
    expect(settings.value.zoom_pct).toBe(125);
  });

  it("keeps the settings when an update fails", async () => {
    const failing = script("known");
    failing.fail = { updateSettings: { code: "io" } };
    await initStores(createMockClient(failing));
    const result = await settings.update({ zoom_pct: 150 });
    expect(result.status).toBe("error");
    expect(settings.value.zoom_pct).toBe(100);
  });

  it("follows the shell's navigate event until unsubscribed", async () => {
    const client = createMockClient(script("known"));
    const stop = await initStores(client);
    client.navigate({ name: "settings" });
    expect(router.route.name).toBe("settings");
    stop();
    client.navigate({ name: "overview" });
    expect(router.route.name).toBe("settings");
    while (router.canGoBack) router.back();
  });
});
