import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import Meter from "./Meter.svelte";

afterEach(cleanup);

describe("Meter", () => {
  it("is a named meter with its range and a plain default value text", () => {
    render(Meter, { usedBp: 5000, severity: "ok", label: "Claude 5-hour" });
    const meter = screen.getByRole("meter", { name: "Claude 5-hour" });
    expect(meter.getAttribute("aria-valuemin")).toBe("0");
    expect(meter.getAttribute("aria-valuemax")).toBe("100");
    expect(meter.getAttribute("aria-valuenow")).toBe("50");
    expect(meter.getAttribute("aria-valuetext")).toBe("50% used");
    expect(meter.hasAttribute("tabindex")).toBe(false);
  });

  it("draws 100 dots and fills as many as the percentage", () => {
    const view = render(Meter, {
      usedBp: 3700,
      severity: "ok",
      label: "x",
    });
    const dots = view.container.querySelectorAll("circle");
    expect(dots.length).toBe(100);
    expect(
      [...dots].filter((dot) => dot.getAttribute("fill") !== "none").length,
    ).toBe(37);
    expect(view.container.querySelector("path")).toBeNull();
  });

  it("marks a raised level with the critical colour past 80 and a glyph", () => {
    const view = render(Meter, {
      usedBp: 9000,
      severity: "warn",
      label: "x",
    });
    const fills = [...view.container.querySelectorAll("circle")].map((c) =>
      c.getAttribute("fill"),
    );
    expect(
      fills.slice(80, 90).every((f) => f === "var(--color-critical-fill)"),
    ).toBe(true);
    expect(fills[79]).toBe("var(--waffle-fill)");
    expect(view.container.querySelector("path")).not.toBeNull();
  });

  it("clamps the range and says no data for an unknown level", () => {
    render(Meter, { usedBp: 20_000, severity: "unknown", label: "u" });
    const meter = screen.getByRole("meter", { name: "u" });
    expect(meter.getAttribute("aria-valuenow")).toBe("100");
    expect(meter.getAttribute("aria-valuetext")).toBe("No data");
  });

  it("takes a full sentence and can be a tab stop", () => {
    render(Meter, {
      usedBp: 100,
      severity: "ok",
      label: "n",
      valueText: "1.0 percent used, resets in 5 minutes",
      focusable: true,
    });
    const meter = screen.getByRole("meter", { name: "n" });
    expect(meter.getAttribute("aria-valuetext")).toBe(
      "1.0 percent used, resets in 5 minutes",
    );
    expect(meter.getAttribute("tabindex")).toBe("0");
  });
});
