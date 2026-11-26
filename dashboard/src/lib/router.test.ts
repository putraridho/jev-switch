import { afterEach, describe, expect, it } from "vitest";
import { routeFromHash, router } from "./router.svelte";

afterEach(() => {
  document.body.replaceChildren();
  while (router.canGoBack) router.back();
});

describe("routeFromHash", () => {
  it("reads the view from the URL fragment", () => {
    expect(routeFromHash("#view=settings")).toEqual({ name: "settings" });
    expect(routeFromHash("#view=provider&provider=codex&theme=dark")).toEqual({
      name: "provider",
      provider: "codex",
    });
  });

  it("falls back to the overview for anything else", () => {
    expect(routeFromHash("#view=provider&provider=nope")).toEqual({
      name: "overview",
    });
    expect(routeFromHash("#view=nowhere")).toEqual({ name: "overview" });
    expect(routeFromHash("")).toEqual({ name: "overview" });
  });
});

describe("router", () => {
  it("goes and comes back, with nothing to leave from the first view", () => {
    expect(router.canGoBack).toBe(false);
    router.go({ name: "settings" });
    expect(router.canGoBack).toBe(true);
    router.go({ name: "provider", provider: "claude" });
    router.back();
    expect(router.route).toEqual({ name: "settings" });
    router.back();
    expect(router.route).toEqual({ name: "overview" });
    router.back();
    expect(router.route).toEqual({ name: "overview" });
  });

  it("remembers the control that opened a view and hands it back once", () => {
    const button = document.createElement("button");
    document.body.append(button);
    button.focus();
    router.go({ name: "settings" });
    router.back();
    expect(router.takeReturnFocus()).toBe(button);
    expect(router.takeReturnFocus()).toBeNull();
  });

  it("remembers no control when only the page had focus", () => {
    router.go({ name: "settings" });
    router.back();
    expect(router.takeReturnFocus()).toBeNull();
  });

  it("hands over the section to focus once", () => {
    router.go({ name: "settings" }, "behaviour");
    expect(router.takeSection()).toBe("behaviour");
    expect(router.takeSection()).toBeNull();
    router.go({ name: "overview" });
    router.back();
    expect(router.takeSection()).toBeNull();
  });
});
