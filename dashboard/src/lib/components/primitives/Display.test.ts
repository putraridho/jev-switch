import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { createRawSnippet, type Snippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import BalanceFigure from "./BalanceFigure.svelte";
import BigNumber from "./BigNumber.svelte";
import EmptyState from "./EmptyState.svelte";
import ErrorState from "./ErrorState.svelte";
import { freshness, type StatusKind } from "./display";
import Freshness from "./Freshness.svelte";
import InlineBanner from "./InlineBanner.svelte";
import ListRow from "./ListRow.svelte";
import LoadingWaffle from "./LoadingWaffle.svelte";
import MeterLabel from "./MeterLabel.svelte";
import NoSource from "./NoSource.svelte";
import PlanTag from "./PlanTag.svelte";
import Section from "./Section.svelte";
import StatusBadge from "./StatusBadge.svelte";
import Toast from "./Toast.svelte";
import WarningLegend from "./WarningLegend.svelte";

const snippet = (text: string): Snippet =>
  createRawSnippet(() => ({ render: () => `<button>${text}</button>` }));

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("display primitives", () => {
  it("words each status", () => {
    const cases: { kind: StatusKind; word: string }[] = [
      { kind: "near_limit", word: "Near limit" },
      { kind: "stale", word: "Stale" },
      { kind: "healthy", word: "Healthy" },
      { kind: "no_source", word: "No source" },
    ];
    for (const c of cases) {
      render(StatusBadge, { kind: c.kind });
      expect(screen.getByText(c.word)).toBeTruthy();
      cleanup();
    }
  });

  it("lays out the label line with name, window and status", () => {
    render(MeterLabel, { name: "Codex", window: "weekly", status: "stale" });
    expect(screen.getByText("Codex")).toBeTruthy();
    expect(screen.getByText("weekly")).toBeTruthy();
    expect(screen.getByText("Stale")).toBeTruthy();
  });

  it("shows a figure with its unit, and a balance with its caption", () => {
    render(BigNumber, { value: "62%", unit: "used", alert: true });
    expect(screen.getByText("used")).toBeTruthy();
    render(BalanceFigure, { amount: "$18.40", caption: "Available" });
    expect(screen.getByText("$18.40")).toBeTruthy();
    render(PlanTag, { plan: "Plus" });
    expect(screen.getByText("Plus")).toBeTruthy();
  });

  it("says an error with its stale age and one action", () => {
    render(ErrorState, {
      staleText: "Stale. Last read 2 hours ago.",
      message: "Balance request timed out.",
      command: snippet("copy"),
      action: snippet("Retry"),
    });
    expect(screen.getByText("Stale. Last read 2 hours ago.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
    render(InlineBanner, {
      title: "DeepSeek timed out",
      body: "Try again in a minute.",
      action: snippet("Again"),
    });
    expect(screen.getByText("DeepSeek timed out")).toBeTruthy();
  });

  it("names providers without a source in one block, or nothing", () => {
    render(NoSource, { names: ["Gemini", "Grok"] });
    expect(screen.getByText("Gemini, Grok")).toBeTruthy();
    cleanup();
    const view = render(NoSource, { names: [] });
    expect(view.container.textContent.trim()).toBe("");
  });

  it("draws an empty state with a ghost row only on request, and a section", () => {
    const view = render(EmptyState, {
      title: "No usage recorded yet.",
      body: "Connected.",
      ghost: true,
      action: snippet("Refresh"),
    });
    expect(view.container.querySelectorAll("circle").length).toBe(10);
    render(Section, {
      label: "Other providers",
      secondary: true,
      children: snippet("x"),
    });
    expect(
      screen.getByRole("heading", { name: "Other providers" }),
    ).toBeTruthy();
  });

  it("loads as 100 outlined dots with one spoken status", () => {
    const view = render(LoadingWaffle, { size: "medium" });
    expect(view.container.querySelectorAll("circle").length).toBe(100);
    expect(screen.getByRole("status").textContent).toContain("Loading usage");
  });

  it("dismisses a toast after four seconds, pausing under the pointer", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const ondismiss = vi.fn();
    render(Toast, { message: "Copied", ondismiss, warning: true });
    const user = userEvent.setup({
      advanceTimers: (ms) => vi.advanceTimersByTime(ms),
    });
    await user.hover(screen.getByRole("status"));
    vi.advanceTimersByTime(5000);
    expect(ondismiss).not.toHaveBeenCalled();
    await user.unhover(screen.getByRole("status"));
    vi.advanceTimersByTime(4000);
    expect(ondismiss).toHaveBeenCalledOnce();
  });

  it("selects a list row, with words instead of a number without a source", async () => {
    const onclick = vi.fn();
    render(ListRow, {
      name: "Codex",
      window: "weekly",
      usedBp: 8300,
      severity: "warn",
      selected: true,
      onclick,
    });
    const row = screen.getByRole("button", { name: /Codex weekly\s*83%/ });
    expect(row.getAttribute("aria-current")).toBe("true");
    await userEvent.click(row);
    expect(onclick).toHaveBeenCalledOnce();
    render(ListRow, { name: "Grok", usedBp: null, onclick });
    expect(screen.getByText("No source Flashboard can read yet.")).toBeTruthy();
  });

  it("words the age of a reading and turns stale after five minutes", () => {
    expect(freshness(0, 59_000)).toEqual({
      text: "Updated 59 s ago",
      stale: false,
    });
    expect(freshness(0, 300_000).stale).toBe(false);
    expect(freshness(0, 301_000)).toEqual({
      text: "Updated 5 min ago",
      stale: true,
    });
    expect(freshness(0, 7_200_000).text).toBe("Updated 2 h ago");
    render(Freshness, { atMs: 0 });
    expect(screen.getByText(/Updated \d+ h ago/)).toBeTruthy();
    render(Freshness, { atMs: Date.now() });
    expect(screen.getByText(/Updated 0/)).toBeTruthy();
    render(WarningLegend, { warnAtPct: 75 });
    expect(screen.getByText("75% warning mark")).toBeTruthy();
  });
});
