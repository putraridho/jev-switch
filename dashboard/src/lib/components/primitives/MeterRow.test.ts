import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { closeTopmost } from "../../escape";
import type { WindowMeterView } from "../../format";
import { FIXTURE_NOW_MS } from "../../ipc/fixtures";
import MeterRow from "./MeterRow.svelte";

const meter: WindowMeterView = {
  kind: "window",
  id: "w:5h",
  window: { kind: "five_hour" },
  used_bp: 5000,
  resets_at: Math.round(FIXTURE_NOW_MS / 1000) + 134 * 60,
  status: "allowed",
  severity: "ok",
  phase: "current",
};
const observedAt = FIXTURE_NOW_MS - 60_000;

function row(patch: Partial<WindowMeterView> = {}) {
  return render(MeterRow, {
    provider: "claude",
    meter: { ...meter, ...patch },
    observedAt,
  });
}

describe("MeterRow", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(FIXTURE_NOW_MS);
    // The clock store read the real time when it loaded; a visibility change resyncs it.
    document.dispatchEvent(new Event("visibilitychange"));
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("shows the name, the number and one quiet line, named for its provider", () => {
    row();
    expect(screen.getByText("5-hour")).toBeTruthy();
    expect(screen.getByText("50%")).toBeTruthy();
    expect(
      screen.getByText(/^Resets in 2h 14m · (today|tomorrow) /),
    ).toBeTruthy();
    expect(screen.getByRole("meter", { name: "Claude 5-hour" })).toBeTruthy();
  });

  it("carries the whole level-1 detail in its accessible text", () => {
    row();
    const text = screen
      .getByRole("meter", { name: "Claude 5-hour" })
      .getAttribute("aria-valuetext");
    expect(text).toMatch(/^50\.0 percent used, resets in 2 hours 14 minutes, /);
    expect(text).toMatch(/status allowed, seen at \d{1,2}:\d{2}/);
  });

  it("is reachable with Tab alone", async () => {
    row();
    await userEvent.tab();
    expect(document.activeElement).toBe(
      screen.getByRole("meter", { name: "Claude 5-hour" }),
    );
  });

  it("opens the detail on keyboard focus and closes it when focus leaves", async () => {
    row();
    expect(screen.queryByText("50.0 %")).toBeNull();
    await userEvent.tab();
    expect(screen.getByText("50.0 %")).toBeTruthy();
    expect(screen.getByText("Allowed")).toBeTruthy();
    expect(screen.getByText("Exact reset")).toBeTruthy();
    expect(screen.getByText("Seen at")).toBeTruthy();
    await userEvent.tab();
    expect(screen.queryByText("50.0 %")).toBeNull();
  });

  it("opens the same detail on hover and keeps it while the pointer is over it", async () => {
    row();
    const meterEl = screen.getByRole("meter", { name: "Claude 5-hour" });
    await userEvent.hover(meterEl);
    const detail = screen.getByText("50.0 %");
    await userEvent.hover(detail);
    expect(screen.getByText("50.0 %")).toBeTruthy();
    await userEvent.unhover(detail);
    expect(screen.queryByText("50.0 %")).toBeNull();
  });

  it("closes on Escape without moving focus, and reopens after focus leaves and returns", async () => {
    row();
    await userEvent.tab();
    const meterEl = screen.getByRole("meter", { name: "Claude 5-hour" });
    expect(closeTopmost()).toBe(true);
    await tick();
    expect(screen.queryByText("50.0 %")).toBeNull();
    expect(document.activeElement).toBe(meterEl);
    expect(closeTopmost()).toBe(false);
    await userEvent.tab();
    await userEvent.tab({ shift: true });
    expect(screen.getByText("50.0 %")).toBeTruthy();
  });

  it("stays closed after Escape until the pointer and focus have both left", async () => {
    row();
    const meterEl = screen.getByRole("meter", { name: "Claude 5-hour" });
    await userEvent.hover(meterEl);
    closeTopmost();
    await tick();
    expect(screen.queryByText("50.0 %")).toBeNull();
    await userEvent.unhover(meterEl);
    await userEvent.hover(meterEl);
    expect(screen.getByText("50.0 %")).toBeTruthy();
  });

  it("invents no number for an elapsed window", () => {
    row({
      phase: "elapsed",
      used_bp: 0,
      resets_at: meter.resets_at === null ? null : meter.resets_at - 3 * 3600,
    });
    expect(screen.getByText("—")).toBeTruthy();
    expect(
      screen.getByText(/^Reset .* · waiting for your next request$/),
    ).toBeTruthy();
  });

  it("says so when the reset time is unknown", async () => {
    row({ resets_at: null });
    expect(screen.getByText("Reset time unknown")).toBeTruthy();
    await userEvent.tab();
    expect(screen.getAllByText("Reset time unknown").length).toBe(2);
  });

  it("shows the level of a rejected window in words", async () => {
    row({ status: "rejected", severity: "critical", used_bp: 10_000 });
    await userEvent.tab();
    expect(screen.getByText("Limit reached")).toBeTruthy();
    expect(screen.getByText("Near limit")).toBeTruthy();
    expect(screen.getByText(/^Past the 80% mark\. Resets in /)).toBeTruthy();
  });

  it("keeps the last good value while stale, without alert or reset time", () => {
    render(MeterRow, {
      provider: "codex",
      meter: { ...meter, used_bp: 9000, severity: "warn" },
      observedAt,
      heading: "Codex",
      size: "medium",
      stale: true,
    });
    expect(screen.getByText("90%")).toBeTruthy();
    expect(screen.getByText("Last good read 1m ago")).toBeTruthy();
    expect(screen.queryByText(/Resets in/)).toBeNull();
    expect(screen.queryByText("Near limit")).toBeNull();
    expect(
      screen
        .getByRole("meter", { name: "Codex 5-hour" })
        .getAttribute("aria-valuetext"),
    ).toMatch(/, stale, last good read 1m ago$/);
  });
});
