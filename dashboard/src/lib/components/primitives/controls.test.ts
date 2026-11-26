import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { closeTopmost } from "../../escape";
import ConfirmDialog from "./ConfirmDialog.svelte";
import NumberField from "./NumberField.svelte";
import RangePills from "./RangePills.svelte";
import SelectField from "./SelectField.svelte";
import Switch from "./Switch.svelte";

afterEach(cleanup);

describe("Switch", () => {
  it("is a switch named by its label and described by its hint", () => {
    render(Switch, {
      label: "Codex",
      hint: "5-hour and weekly windows.",
      checked: true,
      strong: true,
      onchange: () => undefined,
    });
    const control = screen.getByRole("switch", { name: "Codex" });
    expect(control).toHaveProperty("checked", true);
    const id = control.getAttribute("aria-describedby") ?? "";
    expect(document.getElementById(id)?.textContent).toBe(
      "5-hour and weekly windows.",
    );
  });

  it("toggles from anywhere on the row and from the keyboard", async () => {
    const onchange =
      vi.fn<(checked: boolean, input: HTMLInputElement) => void>();
    render(Switch, {
      label: "Start at login",
      checked: false,
      placement: "start",
      onchange,
    });
    await userEvent.click(screen.getByText("Start at login"));
    screen.getByRole("switch").focus();
    await userEvent.keyboard(" ");
    expect(onchange.mock.calls.map((call) => call[0])).toEqual([true, false]);
    expect(screen.getByRole("switch").hasAttribute("aria-describedby")).toBe(
      false,
    );
  });

  it("does nothing when disabled", async () => {
    const onchange = vi.fn<(checked: boolean) => void>();
    render(Switch, { label: "x", checked: false, disabled: true, onchange });
    await userEvent.click(screen.getByText("x"));
    expect(onchange).not.toHaveBeenCalled();
  });
});

describe("NumberField", () => {
  it("is labelled, shows help, and reports change and blur", async () => {
    const onchange = vi.fn<(input: HTMLInputElement) => void>();
    const onblur = vi.fn<(input: HTMLInputElement) => void>();
    render(NumberField, {
      id: "warn",
      label: "Warning (%)",
      value: 80,
      help: "50 to 100",
      onchange,
      onblur,
    });
    const input = screen.getByRole("spinbutton", { name: "Warning (%)" });
    expect(input.getAttribute("aria-describedby")).toBe("warn-help");
    await userEvent.clear(input);
    await userEvent.type(input, "70");
    await userEvent.tab();
    expect(onchange).toHaveBeenCalled();
    expect(onblur).toHaveBeenCalled();
  });

  it("ties the error to the field and marks it invalid", () => {
    render(NumberField, {
      id: "warn",
      label: "Warning (%)",
      value: 99,
      error: "Warn must be below critical (95)",
      onchange: () => undefined,
    });
    const input = screen.getByRole("spinbutton");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("aria-describedby")).toBe("warn-error");
    expect(screen.getByText("Warn must be below critical (95)").id).toBe(
      "warn-error",
    );
  });

  it("says Saved, and has no description without help or error", async () => {
    render(NumberField, {
      id: "z",
      label: "Zoom (%)",
      value: 100,
      saved: true,
      onchange: () => undefined,
    });
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(
      screen.getByRole("spinbutton").hasAttribute("aria-describedby"),
    ).toBe(false);
    await userEvent.tab();
    await userEvent.tab();
  });
});

describe("SelectField", () => {
  it("is labelled and reports the chosen value", async () => {
    const onchange = vi.fn<(value: string) => void>();
    render(SelectField, {
      id: "theme",
      label: "Theme",
      value: "system",
      options: [
        { value: "system", label: "System" },
        { value: "dark", label: "Dark" },
      ],
      onchange,
    });
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: "Theme" }),
      "dark",
    );
    expect(onchange).toHaveBeenCalledWith("dark");
  });
});

describe("RangePills", () => {
  const options = [
    { value: "day", label: "1D" },
    { value: "week", label: "1W" },
  ];

  it("is a named radio group with one choice", async () => {
    const onchange = vi.fn<(value: string) => void>();
    render(RangePills, {
      legend: "Range",
      options,
      value: "week",
      onchange,
    });
    expect(screen.getByRole("group", { name: "Range" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "1W" })).toHaveProperty(
      "checked",
      true,
    );
    await userEvent.click(screen.getByRole("radio", { name: "1D" }));
    expect(onchange).toHaveBeenCalledWith("day");
  });

  it("can be disabled as a whole", async () => {
    const onchange = vi.fn<(value: string) => void>();
    render(RangePills, {
      legend: "Range",
      options,
      value: "week",
      disabled: true,
      onchange,
    });
    await userEvent.click(screen.getByRole("radio", { name: "1D" }));
    expect(onchange).not.toHaveBeenCalled();
  });
});

describe("ConfirmDialog", () => {
  const pressed: ("confirm" | "cancel")[] = [];

  function open(): void {
    pressed.length = 0;
    render(ConfirmDialog, {
      title: "Turn it on?",
      body: "It costs a little.",
      confirmLabel: "Turn on",
      onconfirm: () => {
        pressed.push("confirm");
      },
      oncancel: () => {
        pressed.push("cancel");
      },
    });
  }

  it("is named by its title and starts focus on Cancel", () => {
    open();
    expect(screen.getByRole("dialog", { name: "Turn it on?" })).toBeTruthy();
    expect(document.activeElement?.textContent.trim()).toBe("Cancel");
  });

  it("confirms, cancels, and closes on Esc only while mounted", async () => {
    open();
    await userEvent.click(screen.getByRole("button", { name: "Turn on" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(closeTopmost()).toBe(true);
    expect(pressed).toEqual(["confirm", "cancel", "cancel"]);
    cleanup();
    expect(closeTopmost()).toBe(false);
  });
});
