import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { router } from "../../router.svelte";
import Header from "./Header.svelte";

afterEach(() => {
  cleanup();
  while (router.canGoBack) router.back();
});

describe("Header", () => {
  it("names the app and Settings in words, and marks the current view", () => {
    render(Header);
    const home = screen.getByRole("button", { name: "Flashboard" });
    expect(home.getAttribute("aria-current")).toBe("page");
    expect(
      screen
        .getByRole("button", { name: "Settings" })
        .hasAttribute("aria-current"),
    ).toBe(false);
  });

  it("reaches Settings and comes back", async () => {
    render(Header);
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(router.route.name).toBe("settings");
    expect(
      screen
        .getByRole("button", { name: "Settings" })
        .getAttribute("aria-current"),
    ).toBe("page");
    await userEvent.click(screen.getByRole("button", { name: "Flashboard" }));
    expect(router.route.name).toBe("overview");
  });

  it("does not stack history when the view is already open", async () => {
    render(Header);
    await userEvent.click(screen.getByRole("button", { name: "Flashboard" }));
    expect(router.canGoBack).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    router.back();
    expect(router.route.name).toBe("overview");
  });
});
