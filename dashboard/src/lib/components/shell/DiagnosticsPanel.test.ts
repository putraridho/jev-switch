import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { fixtures } from "../../ipc/fixtures";
import { createMockClient, type MockIpcClient } from "../../ipc/mock";
import type { Diagnostics } from "../../ipc/bindings/Diagnostics";
import { initStores } from "../../stores";
import DiagnosticsPanel from "./DiagnosticsPanel.svelte";

afterEach(cleanup);

describe("DiagnosticsPanel", () => {
  it("shows the version, the history size and each real source in words", async () => {
    const client = createMockClient(structuredClone(fixtures.known));
    await initStores(client);
    render(DiagnosticsPanel);
    expect(
      await screen.findByText(
        /Flashboard 0\.1\.0 · history 1\.2 MB, 8.412 samples/,
      ),
    ).toBeTruthy();
    expect(screen.getByText("Claude gateway log")).toBeTruthy();
    expect(
      screen.getByText(/last problem: offline · 2 failed in a row/),
    ).toBeTruthy();
    expect(screen.getByText("DeepSeek balance")).toBeTruthy();
    expect(screen.getByText("no success yet")).toBeTruthy();
    expect(screen.queryByText(/Gemini/)).toBeNull();
    expect(screen.getByText(/Tray: available/)).toBeTruthy();
  });

  it("says when there is no tray, and asks again on Refresh", async () => {
    const script = structuredClone(fixtures.known);
    const noTray: Diagnostics = {
      ...script.diagnostics,
      tray_available: false,
    };
    script.diagnostics = noTray;
    const client: MockIpcClient = createMockClient(script);
    const spy = vi.spyOn(client, "getDiagnostics");
    await initStores(client);
    render(DiagnosticsPanel);
    expect(await screen.findByText(/Tray: not available/)).toBeTruthy();
    await userEvent.click(
      screen.getByRole("button", { name: "Refresh", hidden: true }),
    );
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("offers a Retry when the diagnostics cannot be read", async () => {
    const client = createMockClient(structuredClone(fixtures.known));
    await initStores(client);
    const spy = vi
      .spyOn(client, "getDiagnostics")
      .mockResolvedValueOnce({ status: "error", error: { code: "internal" } });
    render(DiagnosticsPanel);
    expect(
      await screen.findByText("Diagnostics aren't available right now."),
    ).toBeTruthy();
    await userEvent.click(
      screen.getByRole("button", { name: "Retry", hidden: true }),
    );
    expect(await screen.findByText(/Flashboard 0\.1\.0/)).toBeTruthy();
    expect(spy).toHaveBeenCalledTimes(2);
  });
});
