import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fixtures, type FixtureName } from "../../ipc/fixtures";
import { createMockClient, type MockScript } from "../../ipc/mock";
import { initStores, settings } from "../../stores";
import FirstRun from "./FirstRun.svelte";

function script(name: FixtureName): MockScript {
  const base = structuredClone(fixtures[name]);
  base.settings = { ...base.settings, first_run_seen: false };
  return base;
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  cleanup();
  vi.advanceTimersByTime(5_000);
  vi.useRealTimers();
});

describe("FirstRun", () => {
  it("says what was found in words, names the placeholders and starts with the heading", async () => {
    const client = createMockClient(script("mixed"));
    await initStores(client);
    render(FirstRun);
    expect(
      screen.getByRole("heading", { level: 1, name: "Usage, in one place." }),
    ).toBeTruthy();
    expect(screen.getByText(/Claude · found/)).toBeTruthy();
    expect(screen.getByText(/Codex · found/)).toBeTruthy();
    expect(screen.getByText(/DeepSeek · add a key/)).toBeTruthy();
    expect(
      screen.getByText(
        "Gemini, Antigravity and Grok have no usage source yet.",
      ),
    ).toBeTruthy();
    expect(screen.getByText(/waits in the tray/)).toBeTruthy();
  });

  it("has a line for every state a provider can be in", async () => {
    const client = createMockClient(script("unavailable"));
    await initStores(client);
    render(FirstRun);
    expect(screen.getAllByRole("listitem").length).toBeGreaterThan(0);
    cleanup();
    await initStores(createMockClient(script("first_run")));
    render(FirstRun);
    expect(screen.getByText(/Claude · checking/)).toBeTruthy();
  });

  it("applies Start at login at once and puts it back when saving fails", async () => {
    const failing = script("known");
    failing.fail = { updateSettings: { code: "io" } };
    await initStores(createMockClient(failing));
    render(FirstRun);
    const box = screen.getByRole("checkbox", { name: "Start at login" });
    expect(box).toHaveProperty("checked", true);
    await userEvent.click(box);
    await waitFor(() => {
      expect(box).toHaveProperty("checked", true);
    });
    cleanup();

    const client = createMockClient(script("known"));
    await initStores(client);
    render(FirstRun);
    await userEvent.click(
      screen.getByRole("checkbox", { name: "Start at login" }),
    );
    await waitFor(() => {
      expect(settings.value.autostart).toBe(false);
    });
  });

  it("Continue records that it was seen and moves focus to the overview heading", async () => {
    const client = createMockClient(script("known"));
    await initStores(client);
    const heading = document.createElement("h1");
    heading.id = "view-title";
    heading.tabIndex = -1;
    document.body.append(heading);
    render(FirstRun);
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() => {
      expect(settings.value.first_run_seen).toBe(true);
    });
    heading.remove();
  });

  it("stays put when saving fails", async () => {
    const failing = script("known");
    failing.fail = { updateSettings: { code: "io" } };
    await initStores(createMockClient(failing));
    render(FirstRun);
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(settings.value.first_run_seen).toBe(false);
  });
});
