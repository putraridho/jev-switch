import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { closeTopmost } from "../../escape";
import AppHeader from "./AppHeader.svelte";
import Checkbox from "./Checkbox.svelte";
import ConfirmDialog from "./ConfirmDialog.svelte";
import IconButton from "./IconButton.svelte";
import Menu from "./Menu.svelte";
import RadioGroup from "./RadioGroup.svelte";
import RangePills from "./RangePills.svelte";
import TextTabs from "./TextTabs.svelte";
import Tooltip from "./Tooltip.svelte";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const options = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
  { value: "c", label: "Gamma" },
];

describe("Checkbox", () => {
  it("is named, described, mixed and reports changes", async () => {
    const onchange = vi.fn<(checked: boolean) => void>();
    render(Checkbox, {
      label: "Codex",
      hint: "Two windows.",
      checked: false,
      mixed: true,
      onchange,
    });
    const box = screen.getByRole("checkbox", { name: "Codex" });
    expect(box).toHaveProperty("indeterminate", true);
    expect(box.getAttribute("aria-describedby")).not.toBeNull();
    await userEvent.click(screen.getByText("Codex"));
    expect(onchange).toHaveBeenCalledWith(true);
  });
});

describe("RadioGroup", () => {
  it("is a named group that reports the picked value", async () => {
    const onchange = vi.fn<(value: string) => void>();
    render(RadioGroup, { legend: "Window", options, value: "a", onchange });
    expect(screen.getByRole("group", { name: "Window" })).toBeTruthy();
    await userEvent.click(screen.getByText("Beta"));
    expect(onchange).toHaveBeenCalledWith("b");
  });
});

describe("segmented RangePills", () => {
  it("draws counts inside the option's name", () => {
    render(RangePills, {
      legend: "Providers",
      track: true,
      options: [{ value: "a", label: "All", count: 6 }],
      value: "a",
      onchange: () => undefined,
    });
    expect(screen.getByRole("radio", { name: "All 6" })).toBeTruthy();
  });
});

describe("TextTabs", () => {
  it("selects with arrows, Home, End and click", async () => {
    const onchange = vi.fn<(value: string) => void>();
    render(TextTabs, {
      label: "Sections",
      tabs: options,
      value: "a",
      onchange,
      children: createRawSnippet<[string]>((value) => ({
        render: () => `<p>${value()}</p>`,
      })),
    });
    const first = screen.getByRole("tab", { name: "Alpha" });
    expect(first.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tabpanel").textContent).toContain("a");
    first.focus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(onchange).toHaveBeenLastCalledWith("c");
    screen.getByRole("tab", { name: "Alpha" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(onchange).toHaveBeenLastCalledWith("b");
    await userEvent.keyboard("{End}");
    expect(onchange).toHaveBeenLastCalledWith("c");
    await userEvent.keyboard("{Home}");
    expect(onchange).toHaveBeenLastCalledWith("a");
    await userEvent.keyboard("x");
    await userEvent.click(screen.getByRole("tab", { name: "Beta" }));
    expect(onchange).toHaveBeenLastCalledWith("b");
  });
});

describe("Tooltip", () => {
  const target = createRawSnippet<[string]>((id) => ({
    render: () => `<button aria-describedby="${id()}">Dot</button>`,
  }));

  it("opens after the delay on focus and closes on Esc", async () => {
    vi.useFakeTimers();
    render(Tooltip, { text: "Dot 62 of 100", children: target });
    const tip = screen.getByRole("tooltip", { hidden: true });
    screen.getByRole("button").focus();
    await vi.advanceTimersByTimeAsync(299);
    expect(tip.classList.contains("open")).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(tip.classList.contains("open")).toBe(true);
    expect(closeTopmost()).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(tip.classList.contains("open")).toBe(false);
  });

  it("closes on scroll and when focus leaves", async () => {
    vi.useFakeTimers();
    render(Tooltip, { text: "Tip", children: target, delayMs: 0 });
    const tip = screen.getByRole("tooltip", { hidden: true });
    const button = screen.getByRole("button");
    button.focus();
    await vi.advanceTimersByTimeAsync(1);
    window.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(0);
    expect(tip.classList.contains("open")).toBe(false);
    button.blur();
    button.focus();
    await vi.advanceTimersByTimeAsync(1);
    expect(tip.classList.contains("open")).toBe(true);
    button.blur();
    await vi.advanceTimersByTimeAsync(0);
    expect(tip.classList.contains("open")).toBe(false);
  });
});

describe("Tooltip placement", () => {
  it("stays inside the frame at either edge", async () => {
    vi.useFakeTimers();
    const target = createRawSnippet<[string]>((id) => ({
      render: () => `<button aria-describedby="${id()}">Dot</button>`,
    }));
    render(Tooltip, { text: "Tip", children: target, delayMs: 0 });
    const tip = screen.getByRole("tooltip", { hidden: true });
    const anchor = tip.parentElement;
    if (anchor === null) throw new Error("no anchor");
    Object.defineProperty(tip, "offsetWidth", { value: 100 });
    Object.defineProperty(document.documentElement, "clientWidth", {
      value: 320,
      configurable: true,
    });
    const at = (left: number): void => {
      vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue(
        new DOMRect(left, 100, 20, 20),
      );
    };
    const button = screen.getByRole("button");
    const cases: readonly { left: number; name: string }[] = [
      { left: 0, name: "start" },
      { left: 300, name: "end" },
      { left: 150, name: "tip" },
    ];
    for (const entry of cases) {
      at(entry.left);
      button.blur();
      button.focus();
      await vi.advanceTimersByTimeAsync(1);
      expect(tip.classList.contains(entry.name)).toBe(true);
    }
  });
});

describe("Menu", () => {
  it("moves with arrows, runs an item and returns focus", async () => {
    const first = vi.fn<() => void>();
    render(Menu, {
      label: "More",
      items: [
        { label: "Refresh", shortcut: "R", onselect: first },
        {
          label: "Hide",
          destructive: true,
          separated: true,
          onselect: () => undefined,
        },
      ],
    });
    const button = screen.getByRole("button", { name: "More" });
    await userEvent.click(button);
    const items = screen.getAllByRole("menuitem");
    expect(document.activeElement).toBe(items[0]);
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(items[1]);
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(items[0]);
    await userEvent.keyboard("{ArrowUp}{Home}{End}{Home}{Enter}");
    expect(first).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(button);
  });

  it("closes on Esc, outside press, Tab and its own button", async () => {
    render(Menu, {
      label: "More",
      items: [{ label: "Refresh", onselect: () => undefined }],
    });
    const button = screen.getByRole("button", { name: "More" });
    await userEvent.click(button);
    expect(closeTopmost()).toBe(true);
    await vi.waitFor(() => {
      expect(screen.queryByRole("menu")).toBeNull();
    });
    await userEvent.click(button);
    await userEvent.click(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
    await userEvent.click(button);
    await userEvent.tab();
    expect(screen.queryByRole("menu")).toBeNull();
    await userEvent.click(button);
    await userEvent.click(button);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("IconButton and AppHeader", () => {
  it("an icon button is named by its label", async () => {
    const onclick = vi.fn<() => void>();
    render(IconButton, {
      label: "Refresh",
      icon: "refresh",
      size: "small",
      onclick,
    });
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onclick).toHaveBeenCalledOnce();
  });

  it("the header shows the wordmark and the legend", () => {
    render(AppHeader, { warnAtPct: 70 });
    expect(screen.getByText("Flashboard")).toBeTruthy();
    expect(screen.getByText("70% warning mark")).toBeTruthy();
  });
});

describe("info dialog", () => {
  it("has one button, which takes focus", () => {
    render(ConfirmDialog, {
      title: "About",
      body: "Text.",
      confirmLabel: "Got it",
      cancelLabel: null,
      onconfirm: () => undefined,
      oncancel: () => undefined,
    });
    expect(screen.getAllByRole("button", { hidden: true })).toHaveLength(1);
    expect(document.activeElement?.textContent).toBe("Got it");
  });
});
