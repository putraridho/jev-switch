import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { closeTopmost } from "../lib/escape";
import { fixtures } from "../lib/ipc/fixtures";
import { createMockClient } from "../lib/ipc/mock";
import { announce, initStores, settings } from "../lib/stores";
import Settings from "./Settings.svelte";

beforeEach(async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  await initStores(createMockClient(structuredClone(fixtures.known)));
});
afterEach(() => {
  cleanup();
  vi.advanceTimersByTime(5_000);
  vi.useRealTimers();
});

function field(name: string): HTMLInputElement {
  const found = screen.getByLabelText(name);
  if (!(found instanceof HTMLInputElement))
    throw new Error(`${name} is not an input`);
  return found;
}

async function type(input: HTMLInputElement, value: string): Promise<void> {
  await userEvent.clear(input);
  await userEvent.type(input, value);
  await userEvent.tab();
}

describe("Settings thresholds", () => {
  it("saves a valid warn level, says Saved next to the field and announces it", async () => {
    render(Settings);
    await type(field("Warning (%)"), "70");
    await waitFor(() => {
      expect(settings.value.warn_bp).toBe(7000);
    });
    expect(screen.getByText("Saved")).toBeTruthy();
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toContain("Saved");
    expect(field("Warning (%)").getAttribute("aria-invalid")).toBe("false");
  });

  it("explains a warn level at critical, ties the text to the field and puts the value back", async () => {
    render(Settings);
    const warn = field("Warning (%)");
    await type(warn, "96");
    const message = await screen.findByText("Warn must be below critical (95)");
    expect(warn.getAttribute("aria-invalid")).toBe("true");
    expect(warn.getAttribute("aria-describedby")).toBe(message.id);
    expect(warn.value).toBe("80");
    expect(settings.value.warn_bp).toBe(8000);
    await type(warn, "60");
    await waitFor(() => {
      expect(screen.queryByText(/Warn must be below/)).toBeNull();
    });
  });

  it("explains a critical level at or below warn, and a value out of range", async () => {
    render(Settings);
    const critical = field("Critical (%)");
    await type(critical, "80");
    expect(
      await screen.findByText("Critical must be above warn (80)"),
    ).toBeTruthy();
    expect(critical.getAttribute("aria-describedby")).toBe("critical-error");
    await type(critical, "120");
    expect(
      await screen.findByText("Enter a whole number from 50 to 100"),
    ).toBeTruthy();
    await type(critical, "99");
    await waitFor(() => {
      expect(settings.value.critical_bp).toBe(9900);
    });
    expect(screen.getAllByText("Saved").length).toBeGreaterThan(0);
  });
});

describe("Settings Claude live check", () => {
  it("asks first, keeps it off on Cancel and returns focus to the switch", async () => {
    render(Settings);
    const box = screen.getByRole("switch", { name: /Claude live check/ });
    await userEvent.click(box);
    expect(
      await screen.findByText("Turn on the Claude live check?"),
    ).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(screen.queryByText("Turn on the Claude live check?")).toBeNull();
    });
    expect(box).toHaveProperty("checked", false);
    expect(settings.value.claude_live_check).toBe(false);
    expect(document.activeElement).toBe(box);
  });

  it("closes on Escape before anything else", async () => {
    render(Settings);
    await userEvent.click(
      screen.getByRole("switch", { name: /Claude live check/ }),
    );
    await screen.findByText("Turn on the Claude live check?");
    expect(closeTopmost()).toBe(true);
    await waitFor(() => {
      expect(screen.queryByText("Turn on the Claude live check?")).toBeNull();
    });
  });

  it("turns it on after the confirmation and off again without asking", async () => {
    render(Settings);
    const box = screen.getByRole("switch", { name: /Claude live check/ });
    await userEvent.click(box);
    await userEvent.click(
      await screen.findByRole("button", { name: "Turn on" }),
    );
    await waitFor(() => {
      expect(settings.value.claude_live_check).toBe(true);
    });
    await userEvent.click(box);
    await waitFor(() => {
      expect(settings.value.claude_live_check).toBe(false);
    });
    expect(screen.queryByText("Turn on the Claude live check?")).toBeNull();
  });
});

describe("Settings sections", () => {
  it("lists the diagnostics and every shortcut in text", () => {
    render(Settings);
    expect(screen.getByText("About and diagnostics")).toBeTruthy();
    expect(screen.getByText("Single-key shortcuts")).toBeTruthy();
    expect(screen.getByText("Refresh every provider")).toBeTruthy();
  });

  it("changes the other controls through one update path", async () => {
    render(Settings);
    await userEvent.selectOptions(screen.getByLabelText("Theme"), "dark");
    await waitFor(() => {
      expect(settings.value.theme).toBe("dark");
    });
    await userEvent.click(screen.getByRole("switch", { name: "Grok" }));
    await waitFor(() => {
      expect(settings.value.hidden_providers).toContain("grok");
    });
    await userEvent.click(screen.getByRole("switch", { name: "Grok" }));
    await waitFor(() => {
      expect(settings.value.hidden_providers).not.toContain("grok");
    });
  });

  it("maps a provider switch to the hidden list: off hides, on shows", async () => {
    render(Settings);
    const claude = screen.getByRole("switch", { name: "Claude" });
    expect(claude).toHaveProperty("checked", true);
    await userEvent.click(claude);
    await waitFor(() => {
      expect(settings.value.hidden_providers).toContain("claude");
    });
  });

  it("saves the clock and the tray setting", async () => {
    render(Settings);
    await userEvent.selectOptions(screen.getByLabelText("Clock"), "h24");
    await waitFor(() => {
      expect(settings.value.hour_cycle).toBe("h24");
    });
    const before = settings.value.close_to_tray;
    await userEvent.click(
      screen.getByRole("switch", { name: /Keep running in the tray/ }),
    );
    await waitFor(() => {
      expect(settings.value.close_to_tray).toBe(!before);
    });
  });

  it("disables the level switches while notifications are off", async () => {
    render(Settings);
    await userEvent.click(
      screen.getByRole("switch", { name: "Send notifications" }),
    );
    await waitFor(() => {
      expect(
        screen.getByRole("switch", { name: /warning mark/ }),
      ).toHaveProperty("disabled", true);
    });
    expect(
      screen.getByRole("switch", { name: /critical mark/ }),
    ).toHaveProperty("disabled", true);
  });
});

describe("Settings without a saved change", () => {
  it("puts a switch back when saving fails", async () => {
    const failing = structuredClone(fixtures.known);
    failing.fail = { updateSettings: { code: "io" } };
    await initStores(createMockClient(failing));
    render(Settings);
    const start = screen.getByRole("switch", { name: "Start at login" });
    expect(start).toHaveProperty("checked", true);
    await userEvent.click(start);
    await waitFor(() => {
      expect(start).toHaveProperty("checked", true);
    });
  });
});
