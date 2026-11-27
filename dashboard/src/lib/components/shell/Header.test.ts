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
  const button = (name: string) => screen.getByRole("button", { name });

  it("names the app and all three views in words, and marks the current one", () => {
    render(Header);
    expect(screen.getByText("Flashboard")).toBeTruthy();
    expect(button("Usage").getAttribute("aria-current")).toBe("page");
    expect(button("Chat").hasAttribute("aria-current")).toBe(false);
    expect(button("Settings").hasAttribute("aria-current")).toBe(false);
    expect(button("Refresh")).toBeTruthy();
  });

  it("reaches Chat and Settings and comes back; Refresh belongs to the overview", async () => {
    render(Header);
    await userEvent.click(button("Chat"));
    expect(router.route.name).toBe("chat");
    expect(button("Chat").getAttribute("aria-current")).toBe("page");
    expect(screen.queryByRole("button", { name: "Refresh" })).toBeNull();
    await userEvent.click(button("Settings"));
    expect(router.route.name).toBe("settings");
    expect(button("Settings").getAttribute("aria-current")).toBe("page");
    await userEvent.click(button("Usage"));
    expect(router.route.name).toBe("overview");
  });

  it("does not stack history when the view is already open", async () => {
    render(Header);
    await userEvent.click(button("Usage"));
    expect(router.canGoBack).toBe(false);
    await userEvent.click(button("Settings"));
    await userEvent.click(button("Settings"));
    router.back();
    expect(router.route.name).toBe("overview");
  });
});
