import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { announce } from "../../stores/announce.svelte";
import CommandLine from "./CommandLine.svelte";
import CopyButton from "./CopyButton.svelte";

const write = vi.fn<(text: string) => Promise<void>>();

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  write.mockReset();
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: write },
    configurable: true,
  });
});
afterEach(() => {
  cleanup();
  vi.advanceTimersByTime(5_000);
  vi.useRealTimers();
});

describe("CopyButton", () => {
  it("shows Copy, names what it copies and starts with the visible word", () => {
    render(CopyButton, { text: "codex login" });
    const button = screen.getByRole("button", { name: "Copy codex login" });
    expect(button.textContent.trim()).toBe("Copy");
  });

  it("copies, says Copied once and goes back to Copy", async () => {
    write.mockResolvedValue();
    render(CopyButton, { text: "claude auth login" });
    await userEvent.click(screen.getByRole("button"));
    expect(write).toHaveBeenCalledWith("claude auth login");
    expect(
      screen.getByRole("button", { name: "Copied claude auth login" }),
    ).toBeTruthy();
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toContain("Copied");
    await vi.waitFor(() => {
      expect(screen.getByRole("button").textContent.trim()).toBe("Copy");
    });
  });

  it("says so when the clipboard refuses", async () => {
    write.mockRejectedValue(new Error("denied"));
    render(CopyButton, { text: "jev-claude" });
    await userEvent.click(screen.getByRole("button"));
    vi.advanceTimersByTime(2_100);
    expect(announce.current).toContain("Couldn't copy");
    expect(screen.getByRole("button").textContent.trim()).toBe("Copy");
  });

  it("restarts the timer on a second press", async () => {
    write.mockResolvedValue();
    render(CopyButton, { text: "x" });
    await userEvent.click(screen.getByRole("button"));
    await userEvent.click(screen.getByRole("button"));
    expect(write).toHaveBeenCalledTimes(2);
  });
});

describe("CommandLine", () => {
  it("shows the command with its Copy button", () => {
    render(CommandLine, { command: "codex login" });
    expect(screen.getByText("codex login").tagName).toBe("CODE");
    expect(
      screen.getByRole("button", { name: "Copy codex login" }),
    ).toBeTruthy();
  });
});
