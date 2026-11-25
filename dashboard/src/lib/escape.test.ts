import { describe, expect, it, vi } from "vitest";
import { closeTopmost, pushEscape } from "./escape";

describe("escape layers", () => {
  it("does nothing when nothing is open", () => {
    expect(closeTopmost()).toBe(false);
  });

  it("closes a dialog before a disclosure, one layer per press", () => {
    const calls: string[] = [];
    const removeDisclosure = pushEscape("disclosure", () => {
      calls.push("disclosure");
      return true;
    });
    const removeDialog = pushEscape("dialog", () => {
      calls.push("dialog");
      return true;
    });
    expect(closeTopmost()).toBe(true);
    removeDialog();
    expect(closeTopmost()).toBe(true);
    removeDisclosure();
    expect(closeTopmost()).toBe(false);
    expect(calls).toEqual(["dialog", "disclosure"]);
  });

  it("asks the newest handler of a layer first and skips one that closed nothing", () => {
    const older = vi.fn(() => true);
    const newer = vi.fn(() => false);
    const removeOlder = pushEscape("disclosure", older);
    const removeNewer = pushEscape("disclosure", newer);
    expect(closeTopmost()).toBe(true);
    expect(newer).toHaveBeenCalledOnce();
    expect(older).toHaveBeenCalledOnce();
    removeNewer();
    removeOlder();
  });

  it("removing a handler twice is harmless", () => {
    const remove = pushEscape("dialog", () => true);
    remove();
    remove();
    expect(closeTopmost()).toBe(false);
  });
});
