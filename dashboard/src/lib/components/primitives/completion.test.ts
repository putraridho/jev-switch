import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { closeTopmost } from "../../escape";
import type { HistorySeries } from "../../ipc/bindings/HistorySeries";
import DataTable from "./DataTable.svelte";
import type { TableCell, TableColumn } from "./display";
import ListRow from "./ListRow.svelte";
import Meter from "./Meter.svelte";
import { buildTrace, SPARK } from "./sparkline";
import Sparkline from "./Sparkline.svelte";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function series(
  values: readonly (readonly [number, number])[],
  unit: HistorySeries["unit"] = "basis_points",
): HistorySeries {
  return {
    provider: "claude",
    meter_id: "window:five_hour",
    from: 0,
    to: 1_000,
    unit,
    gap_s: 100,
    resets: [],
    points: values.map(([t, v]) => ({ t, v, lo: v, hi: v })),
  };
}

describe("sparkline", () => {
  it("never invents a trace", () => {
    expect(buildTrace(series([]), 80)).toBeNull();
    const view = render(Sparkline, { series: null });
    expect(view.container.querySelector(".empty")).toBeTruthy();
    expect(view.container.querySelector("circle")).toBeNull();
  });

  it("draws ink below the mark with the dot at the right end", () => {
    const trace = buildTrace(
      series([
        [0, 0],
        [50, 5_000],
      ]),
      80,
    );
    expect(trace?.ink).toBe("M2 21 L90 12");
    expect(trace?.alert).toBe("");
    expect(trace?.guide).toBeNull();
    expect(trace?.end).toEqual({ x: SPARK.right, y: 12, alert: false });
  });

  it("changes colour where the line crosses the mark", () => {
    const rising = series([
      [0, 6_000],
      [50, 10_000],
    ]);
    const trace = buildTrace(rising, 80);
    expect(trace?.ink).toBe("M2 10.2 L46 6.6");
    expect(trace?.alert).toBe("M46 6.6 L90 3");
    expect(trace?.guide).toBeCloseTo(6.6);
    expect(trace?.end.alert).toBe(true);
    const view = render(Sparkline, { series: rising });
    expect(view.container.querySelector(".guide")).toBeTruthy();
    expect(view.container.querySelector(".alert")).toBeTruthy();
    expect(
      view.container.querySelector("svg")?.getAttribute("aria-hidden"),
    ).toBe("true");
  });

  it("falls back to ink after dropping under the mark, and keeps long raised runs", () => {
    const trace = buildTrace(
      series([
        [0, 9_000],
        [50, 9_000],
        [100, 1_000],
      ]),
      80,
    );
    expect(trace?.alert.startsWith("M2 ")).toBe(true);
    expect(trace?.ink).not.toBe("");
    expect(trace?.end.alert).toBe(false);
  });

  it("puts a flat series at mid height and leaves gaps open", () => {
    const trace = buildTrace(
      series([
        [0, 3_000],
        [50, 3_000],
        [500, 3_000],
      ]),
      80,
    );
    expect(trace?.end.y).toBe(SPARK.height / 2);
    expect(trace?.ink.match(/M/g)).toHaveLength(1);
  });

  it("gives balances their own scale and no mark, and one point only a dot", () => {
    const balance = buildTrace(
      series(
        [
          [0, 100],
          [50, 200],
        ],
        "micros",
      ),
      80,
    );
    expect(balance?.ink).toBe("M2 21 L90 3");
    expect(balance?.alert).toBe("");
    const single = buildTrace(series([[0, 100]]), 80);
    expect(single?.ink).toBe("");
    expect(single?.end.x).toBe(SPARK.right);
  });

  it("sits in a list row", () => {
    render(ListRow, {
      name: "Codex",
      usedBp: null,
      note: "No data",
      onclick: () => undefined,
      trace: createRawSnippet(() => ({
        render: () => `<span>trace here</span>`,
      })),
    });
    expect(screen.getByText("trace here")).toBeTruthy();
  });
});

const columns: readonly TableColumn[] = [
  { label: "Provider", tone: "strong" },
  { label: "Peak %", numeric: true },
  { label: "Resets", numeric: true, tone: "muted" },
];
const rows: readonly (readonly TableCell[])[] = [
  [{ text: "Codex" }, { text: "83%", tone: "alert" }],
];

describe("data table", () => {
  it("is a named table that marks the sorted column", () => {
    const view = render(DataTable, {
      caption: "Usage history",
      columns,
      rows,
      sortIndex: 1,
      sortDirection: "descending",
    });
    expect(screen.getByRole("table", { name: "Usage history" })).toBeTruthy();
    const sorted = screen.getByRole("columnheader", { name: "Peak %" });
    expect(sorted.getAttribute("aria-sort")).toBe("descending");
    expect(screen.getByRole("cell", { name: "83%" }).className).toContain(
      "alert",
    );
    expect(view.container.querySelectorAll("tbody td")).toHaveLength(3);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("asks for an order from its header buttons", async () => {
    const onsort = vi.fn();
    render(DataTable, { caption: "Usage history", columns, rows, onsort });
    await userEvent.click(screen.getByRole("button", { name: "Resets" }));
    expect(onsort).toHaveBeenCalledWith(2);
  });
});

describe("waffle dot tooltip", () => {
  const props = {
    usedBp: 6_200,
    severity: "ok" as const,
    label: "Claude 5-hour",
    focusable: true,
  };
  const dot = (root: Element, at: number): Element => {
    const found = root.querySelector(`[data-dot="${String(at)}"]`);
    if (found === null) throw new Error("no dot");
    return found;
  };
  const over = (target: Element): void => {
    target.dispatchEvent(new MouseEvent("pointerover", { bubbles: true }));
  };

  it("is off unless asked for", () => {
    const view = render(Meter, props);
    expect(view.container.querySelector("[data-dot]")).toBeNull();
  });

  it("opens after 300 ms, moves at once, and closes on leave", async () => {
    vi.useFakeTimers();
    const view = render(Meter, { ...props, dotTooltip: true });
    over(dot(view.container, 4));
    await vi.advanceTimersByTimeAsync(299);
    expect(screen.queryByText("Dot 5 of 100")).toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(screen.getByText("Dot 5 of 100")).toBeTruthy();
    over(dot(view.container, 99));
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.getByText("Dot 100 of 100")).toBeTruthy();
    // Moving onto the tooltip keeps it (WCAG 1.4.13).
    const bridge = view.container.querySelector(".bridge");
    if (bridge === null) throw new Error("no bridge");
    over(bridge);
    expect(screen.getByText("Dot 100 of 100")).toBeTruthy();
    view.container
      .querySelector("svg")
      ?.dispatchEvent(new MouseEvent("pointerleave"));
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.queryByText("Dot 100 of 100")).toBeNull();
  });

  it("names the value's dot on focus without changing the meter, and closes on Esc and scroll", async () => {
    vi.useFakeTimers();
    render(Meter, { ...props, dotTooltip: true, size: "medium" });
    const meter = screen.getByRole("meter", { name: "Claude 5-hour" });
    meter.focus();
    await vi.advanceTimersByTimeAsync(300);
    expect(screen.getByText("Dot 62 of 100")).toBeTruthy();
    expect(meter.getAttribute("aria-valuetext")).toBe("62% used");
    expect(closeTopmost()).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.queryByText("Dot 62 of 100")).toBeNull();
    expect(document.activeElement).toBe(meter);
    meter.blur();
    meter.focus();
    await vi.advanceTimersByTimeAsync(300);
    window.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(0);
    expect(screen.queryByText("Dot 62 of 100")).toBeNull();
    meter.blur();
  });

  it("names nothing on focus when no dot is filled", async () => {
    vi.useFakeTimers();
    const view = render(Meter, { ...props, usedBp: 0, dotTooltip: true });
    screen.getByRole("meter").focus();
    await vi.advanceTimersByTimeAsync(300);
    expect(view.container.querySelector(".dot-tip")).toBeNull();
  });
});
