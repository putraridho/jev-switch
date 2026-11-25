import { describe, expect, it, vi } from "vitest";
import { providerOrder } from "./copy";
import type { ProviderKey } from "./ipc/bindings/ProviderKey";
import { behaviourId, handleKey, zoomNext, type KeyEnv } from "./shortcuts";

function env(patch: Partial<KeyEnv> = {}): KeyEnv {
  return {
    shortcutsOn: () => true,
    hidden: () => [],
    order: () => providerOrder,
    closeTopmost: () => false,
    canGoBack: () => true,
    back: vi.fn(),
    refresh: vi.fn(),
    openSettings: vi.fn(),
    openProvider: vi.fn(),
    zoom: vi.fn(),
    ...patch,
  };
}

function key(
  k: string,
  init: KeyboardEventInit = {},
  target?: HTMLElement,
): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key: k,
    cancelable: true,
    ...init,
  });
  if (target !== undefined)
    Object.defineProperty(event, "target", { value: target });
  return event;
}

describe("single-key shortcuts", () => {
  it("R refreshes, comma opens settings, ? opens the Behaviour list", () => {
    const e = env();
    expect(handleKey(key("r"), e)).toBe(true);
    expect(handleKey(key("R"), e)).toBe(true);
    expect(handleKey(key(","), e)).toBe(true);
    expect(handleKey(key("?"), e)).toBe(true);
    expect(e.refresh).toHaveBeenCalledTimes(2);
    expect(e.openSettings).toHaveBeenNthCalledWith(1);
    expect(e.openSettings).toHaveBeenNthCalledWith(2, behaviourId);
  });

  it("digits open providers in the fixed order, never the card order", () => {
    const e = env();
    for (const digit of ["1", "2", "3", "4", "5", "6"])
      handleKey(key(digit), e);
    const opened: ProviderKey[] = vi
      .mocked(e.openProvider)
      .mock.calls.map(([provider]) => provider);
    expect(opened).toEqual([...providerOrder]);
  });

  it("ignores a digit with no provider and the digit of a hidden provider", () => {
    const e = env({ hidden: () => ["codex"] });
    expect(handleKey(key("7"), e)).toBe(false);
    expect(handleKey(key("0"), e)).toBe(false);
    expect(handleKey(key("2"), e)).toBe(false);
    expect(e.openProvider).not.toHaveBeenCalled();
  });

  it("does nothing when shortcuts are switched off", () => {
    const e = env({ shortcutsOn: () => false });
    expect(handleKey(key("r"), e)).toBe(false);
    expect(e.refresh).not.toHaveBeenCalled();
  });

  it("does nothing while a text field has focus", () => {
    const e = env();
    const input = document.createElement("input");
    input.type = "number";
    expect(handleKey(key("r", {}, input), e)).toBe(false);
    expect(handleKey(key("r", {}, document.createElement("textarea")), e)).toBe(
      false,
    );
    expect(handleKey(key("r", {}, document.createElement("select")), e)).toBe(
      false,
    );
    const editable = document.createElement("div");
    Object.defineProperty(editable, "isContentEditable", { value: true });
    expect(handleKey(key("r", {}, editable), e)).toBe(false);
    expect(e.refresh).not.toHaveBeenCalled();
  });

  it("still works on a checkbox or a button", () => {
    const e = env();
    const box = document.createElement("input");
    box.type = "checkbox";
    expect(handleKey(key("r", {}, box), e)).toBe(true);
    expect(handleKey(key("r", {}, document.createElement("button")), e)).toBe(
      true,
    );
  });

  it("leaves modified keys and other keys to the browser", () => {
    const e = env();
    expect(handleKey(key("r", { metaKey: true }), e)).toBe(false);
    expect(handleKey(key("r", { altKey: true }), e)).toBe(false);
    expect(handleKey(key("r", { ctrlKey: true }), e)).toBe(false);
    expect(handleKey(key("x"), e)).toBe(false);
    expect(handleKey(key("r", { cancelable: true }), e)).toBe(true);
  });

  it("does not act on an event something else already handled", () => {
    const e = env();
    const event = key("r");
    event.preventDefault();
    expect(handleKey(event, e)).toBe(false);
  });
});

describe("Escape", () => {
  it("closes the topmost layer and does not go back", () => {
    const e = env({ closeTopmost: () => true });
    expect(handleKey(key("Escape"), e)).toBe(true);
    expect(e.back).not.toHaveBeenCalled();
  });

  it("goes back when nothing is open", () => {
    const e = env();
    expect(handleKey(key("Escape"), e)).toBe(true);
    expect(e.back).toHaveBeenCalledOnce();
  });

  it("does nothing on the first view", () => {
    const e = env({ canGoBack: () => false });
    expect(handleKey(key("Escape"), e)).toBe(false);
  });

  it("works with shortcuts off and inside a text field", () => {
    const e = env({ shortcutsOn: () => false });
    expect(
      handleKey(key("Escape", {}, document.createElement("input")), e),
    ).toBe(true);
    expect(e.back).toHaveBeenCalledOnce();
  });
});

describe("zoom keys", () => {
  it("map Ctrl plus, minus and zero, even with shortcuts off", () => {
    const e = env({ shortcutsOn: () => false });
    expect(handleKey(key("+", { ctrlKey: true }), e)).toBe(true);
    expect(handleKey(key("=", { ctrlKey: true }), e)).toBe(true);
    expect(handleKey(key("-", { ctrlKey: true }), e)).toBe(true);
    expect(handleKey(key("_", { ctrlKey: true }), e)).toBe(true);
    expect(handleKey(key("0", { ctrlKey: true }), e)).toBe(true);
    expect(vi.mocked(e.zoom).mock.calls.map(([step]) => step)).toEqual([
      "in",
      "in",
      "out",
      "out",
      "reset",
    ]);
  });

  it("ignore Ctrl with another key or with Alt", () => {
    const e = env();
    expect(handleKey(key("x", { ctrlKey: true }), e)).toBe(false);
    expect(handleKey(key("+", { ctrlKey: true, altKey: true }), e)).toBe(false);
  });

  it("step through 100 to 200 percent in 25s", () => {
    expect(zoomNext(100, "in")).toBe(125);
    expect(zoomNext(200, "in")).toBe(200);
    expect(zoomNext(100, "out")).toBe(100);
    expect(zoomNext(150, "out")).toBe(125);
    expect(zoomNext(175, "reset")).toBe(100);
  });
});
