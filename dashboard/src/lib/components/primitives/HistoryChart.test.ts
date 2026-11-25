import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { fixtures } from "../../ipc/fixtures";
import { stubResizeObserver } from "../../test-support";
import HistoryChart from "./HistoryChart.svelte";

const series = fixtures.known.history;

function chart(
  patch: Partial<Parameters<typeof render<typeof HistoryChart>>[1]> = {},
) {
  return render(HistoryChart, {
    series,
    range: "day",
    name: "5-hour",
    thresholds: { warnBp: 8000, criticalBp: 9500 },
    currency: null,
    ...patch,
  });
}

// jsdom has no ResizeObserver, which `bind:clientWidth` needs.
beforeAll(stubResizeObserver);
afterEach(cleanup);

describe("HistoryChart", () => {
  it("describes the plot as one image and lists the data in a hidden table", () => {
    chart();
    const image = screen.getByRole("img");
    expect(image.getAttribute("aria-label")).toMatch(
      /^5-hour usage over the last day: peak \d+% at .+, now \d+%, \d+ resets$/,
    );
    expect(screen.getAllByRole("row")).toHaveLength(series.points.length + 1);
    expect(screen.getByText("critical 95%")).toBeTruthy();
    expect(screen.getByText("warn 80%")).toBeTruthy();
  });

  it("is one slider that starts at the newest point and moves with the keys", async () => {
    chart();
    const slider = screen.getByRole("slider", { name: "Inspect history" });
    const last = String(series.points.length - 1);
    expect(slider.getAttribute("aria-valuemin")).toBe("0");
    expect(slider.getAttribute("aria-valuemax")).toBe(last);
    expect(slider.getAttribute("aria-valuetext")).toBe("");
    slider.focus();
    await userEvent.keyboard("{Home}");
    expect(slider.getAttribute("aria-valuenow")).toBe("0");
    expect(slider.getAttribute("aria-valuetext")).toMatch(
      /^\d{1,2}:\d\d( [AP]M)?, \d+ percent$/,
    );
    await userEvent.keyboard("{ArrowRight}");
    expect(slider.getAttribute("aria-valuenow")).toBe("1");
    await userEvent.keyboard("{End}");
    expect(slider.getAttribute("aria-valuenow")).toBe(last);
    await userEvent.keyboard("{x}");
    expect(slider.getAttribute("aria-valuenow")).toBe(last);
  });

  it("shows the same readout under the pointer and lets go when it leaves", async () => {
    chart();
    const slider = screen.getByRole("slider");
    expect(screen.getByText(/Hover or focus the chart/)).toBeTruthy();
    await fireEvent.pointerMove(slider, { clientX: 100 });
    expect(screen.queryByText(/Hover or focus the chart/)).toBeNull();
    expect(document.querySelector(".readout")?.textContent).toMatch(/percent/);
    await fireEvent.pointerDown(slider);
    await fireEvent.pointerLeave(slider);
    expect(slider.getAttribute("aria-valuetext")).toMatch(/percent/);
  });

  it("draws a focus ring only while the slider has focus", async () => {
    chart();
    const slider = screen.getByRole("slider");
    slider.focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(slider.classList.contains("focused")).toBe(true);
    slider.blur();
    await Promise.resolve();
    expect(slider.classList.contains("focused")).toBe(false);
  });

  it("words a balance in its currency and draws no thresholds", () => {
    chart({
      series: {
        ...series,
        unit: "micros",
        points: series.points.map((p) => ({
          ...p,
          v: p.v * 1000,
          lo: p.v * 1000,
          hi: p.v * 1000,
        })),
      },
      thresholds: null,
      currency: "cny",
      name: "Balance",
    });
    expect(screen.getByRole("img").getAttribute("aria-label")).toContain(
      "Balance balance over the last day",
    );
    expect(screen.queryByText(/critical/)).toBeNull();
  });

  it("has no slider when there is nothing to inspect", () => {
    chart({ series: { ...series, points: [] } });
    expect(screen.queryByRole("slider")).toBeNull();
    expect(screen.getByRole("img").getAttribute("aria-label")).toContain(
      "no history yet",
    );
  });
});
