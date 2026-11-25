import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { fixtures, type FixtureName } from "../../ipc/fixtures";
import type { MeterView } from "../../ipc/bindings/MeterView";
import {
  createMockClient,
  type MockIpcClient,
  type MockScript,
} from "../../ipc/mock";
import { stubResizeObserver } from "../../test-support";
import { initStores } from "../../stores";
import HistorySection from "./HistorySection.svelte";

function script(name: FixtureName): MockScript {
  return structuredClone(fixtures[name]);
}

const meters: MeterView[] = [
  {
    kind: "window",
    id: "w:5h",
    window: { kind: "five_hour" },
    used_bp: 5000,
    resets_at: 1,
    status: "allowed",
    severity: "ok",
    phase: "current",
  },
  {
    kind: "window",
    id: "w:7d",
    window: { kind: "seven_day" },
    used_bp: 3700,
    resets_at: 1,
    status: "allowed",
    severity: "ok",
    phase: "current",
  },
];

let client: MockIpcClient;

async function open(patch: Partial<MockScript> = {}): Promise<void> {
  client = createMockClient({ ...script("known"), ...patch });
  await initStores(client);
}

function calls(): number {
  return client.calls.filter((c) => c.command === "getHistory").length;
}

beforeAll(stubResizeObserver);
beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
});
afterEach(() => {
  cleanup();
  vi.advanceTimersByTime(5_000);
  vi.useRealTimers();
});

describe("HistorySection", () => {
  it("loads the first meter's history and draws the chart, source line included", async () => {
    await open();
    render(HistorySection, { provider: "claude", meters });
    expect(screen.getByText("Loading history…")).toBeTruthy();
    expect(await screen.findByRole("img")).toBeTruthy();
    expect(calls()).toBe(1);
    expect(
      screen.getByText("Source: gateway log (updates when you use jev-claude)"),
    ).toBeTruthy();
    expect(screen.getByRole("heading", { name: "History" })).toBeTruthy();
  });

  it("offers a meter selector only with more than one meter, and asks again on change", async () => {
    await open();
    const view = render(HistorySection, { provider: "claude", meters });
    await screen.findByRole("img");
    const select = screen.getByLabelText("Meter");
    await userEvent.selectOptions(select, "w:7d");
    await waitFor(() => {
      expect(calls()).toBe(2);
    });
    view.unmount();
    render(HistorySection, { provider: "claude", meters: meters.slice(0, 1) });
    expect(screen.queryByLabelText("Meter")).toBeNull();
  });

  it("is a radio group of four ranges and asks again for the one chosen", async () => {
    await open();
    render(HistorySection, { provider: "claude", meters });
    await screen.findByRole("img");
    const radios = screen.getAllByRole("radio");
    expect(radios.map((r) => r.closest("label")?.textContent.trim())).toEqual([
      "Day",
      "Week",
      "Month",
      "Quarter",
    ]);
    expect(screen.getByRole("radio", { name: "Day" })).toHaveProperty(
      "checked",
      true,
    );
    await userEvent.click(screen.getByRole("radio", { name: "Week" }));
    await waitFor(() => {
      expect(calls()).toBe(2);
    });
    expect(screen.getByRole("radio", { name: "Week" })).toHaveProperty(
      "checked",
      true,
    );
  });

  it("says so when there is no history, and when it cannot be read, with a Retry", async () => {
    const empty = script("known");
    empty.history = { ...empty.history, points: [] };
    await open({ history: empty.history });
    const first = render(HistorySection, { provider: "claude", meters });
    expect(
      await screen.findByText("No history in this range yet."),
    ).toBeTruthy();
    first.unmount();
    cleanup();

    await open({ fail: { getHistory: { code: "io" } } });
    render(HistorySection, { provider: "claude", meters });
    expect(
      await screen.findByText("History isn't available right now."),
    ).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => {
      expect(calls()).toBe(2);
    });
  });

  it("shows a balance without thresholds", async () => {
    await open();
    render(HistorySection, {
      provider: "deepseek",
      meters: [
        {
          kind: "balance",
          id: "b:cny",
          currency: "cny",
          total_micros: 1,
          granted_micros: null,
          topped_up_micros: null,
          usable: true,
          severity: "ok",
        },
      ],
    });
    await screen.findByRole("img");
    expect(screen.queryByText(/critical/)).toBeNull();
    expect(
      screen.getByText("Source: DeepSeek balance (asked every few minutes)"),
    ).toBeTruthy();
  });

  it("offers Check now for Claude only when the live check is on", async () => {
    await open();
    const first = render(HistorySection, { provider: "claude", meters });
    await screen.findByRole("img");
    expect(screen.queryByRole("button", { name: "Check now" })).toBeNull();
    first.unmount();
    cleanup();

    const on = script("known");
    on.settings = { ...on.settings, claude_live_check: true };
    await open({ settings: on.settings });
    render(HistorySection, { provider: "claude", meters });
    await screen.findByRole("img");
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(client.calls.filter((c) => c.command === "refresh")).toHaveLength(1);
  });

  it("drops a slow answer for a range that is no longer the one shown", async () => {
    await open();
    render(HistorySection, { provider: "claude", meters });
    await screen.findByRole("img");
    await userEvent.click(screen.getByRole("radio", { name: "Month" }));
    await userEvent.click(screen.getByRole("radio", { name: "Quarter" }));
    await waitFor(() => {
      expect(screen.getByRole("radio", { name: "Quarter" })).toHaveProperty(
        "checked",
        true,
      );
    });
    expect(await screen.findByRole("img")).toBeTruthy();
  });
});
