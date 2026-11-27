import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { fixtures } from "../lib/ipc/fixtures";
import { createMockClient } from "../lib/ipc/mock";
import { closeTopmost } from "../lib/escape";
import { chat, initChat } from "../lib/stores/chat.svelte";
import Chat from "./Chat.svelte";

afterEach(() => {
  cleanup();
});

describe("Chat screen", () => {
  it("renders controls, opens the folder picker, sends a prompt, and handles permissions", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);
    render(Chat);

    expect(
      screen.getByRole("heading", { name: "What should we work on?" }),
    ).toBeTruthy();

    // Open folder picker and select another folder.
    await userEvent.click(screen.getByRole("button", { name: /Folder:/ }));
    expect(
      screen.getByRole("region", { name: "Choose workspace folder" }),
    ).toBeTruthy();
    const pathInput = screen.getByRole("textbox", { name: "Workspace path" });
    await userEvent.clear(pathInput);
    await userEvent.type(pathInput, "/workspace/demo");
    await userEvent.click(screen.getByRole("button", { name: "Use folder" }));
    expect(screen.getByRole("button", { name: "Folder: demo" })).toBeTruthy();

    // Send a prompt.
    const box = screen.getByRole("textbox", { name: "Prompt" });
    await userEvent.type(box, "Check status");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.getAllByText("Check status").length).toBeGreaterThan(0);
    // The session is named after its first prompt.
    expect(screen.getByRole("heading", { name: "Check status" })).toBeTruthy();
    expect(screen.getByText(/Mock reply in \/workspace\/demo/)).toBeTruthy();

    // Simulate a tool permission request and approve it.
    client.pushChat({
      kind: "permission_requested",
      prompt: {
        request_id: "perm_9",
        tool: "Bash",
        summary: "git status",
        blocked_path: null,
        can_always: true,
        always_label: "Bash(git status:*)",
      },
    });
    const approval = await screen.findByRole("alertdialog", {
      name: "Tool approval required",
    });
    expect(approval.textContent).toContain("wants to use Bash");
    expect(approval.textContent).toContain("git status");
    await userEvent.click(screen.getByRole("button", { name: "Allow once" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();

    stop();
  });

  it("closes the folder picker on Esc and keeps an unsent draft across visits", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);
    const first = render(Chat);
    await userEvent.click(screen.getByRole("button", { name: /Folder:/ }));
    expect(closeTopmost()).toBe(true);
    await Promise.resolve();
    expect(
      screen.queryByRole("region", { name: "Choose workspace folder" }),
    ).toBeNull();
    expect(closeTopmost()).toBe(false);

    await userEvent.type(
      screen.getByRole("textbox", { name: "Prompt" }),
      "half written",
    );
    first.unmount();
    render(Chat);
    const again = screen.getByRole("textbox", { name: "Prompt" });
    if (!(again instanceof HTMLTextAreaElement)) throw new Error("no textarea");
    expect(again.value).toBe("half written");
    chat.draft = "";
    stop();
  });

  it("shows assistant replies as Markdown and the user's text as typed", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);
    const view = render(Chat);
    await userEvent.type(
      screen.getByRole("textbox", { name: "Prompt" }),
      "**not bold**",
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(screen.getAllByText("**not bold**").length).toBeGreaterThan(0);
    client.pushChat({
      kind: "state",
      state: {
        ...chat.state,
        messages: [
          ...chat.state.messages,
          {
            id: 99,
            role: "assistant",
            content: "Done: **built** `ok`",
            provider: "claude",
            model: "m",
            route_reason: null,
            tool_calls: [],
            thinking: null,
            changed_files: [],
            status: "answered",
            usage: null,
          },
        ],
      },
    });
    await Promise.resolve();
    const last = [...view.container.querySelectorAll("article.msg")].at(-1);
    expect(last?.querySelector("strong")?.textContent).toBe("built");
    expect(last?.querySelector("code")?.textContent).toBe("ok");
    stop();
  });

  it("sets the mode from the composer and writes the router's note as a sentence", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);
    render(Chat);
    const plan = screen.getByRole("button", { name: "Plan" });
    await userEvent.click(plan);
    expect(plan.getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("Read only")).toBeTruthy();

    await userEvent.type(
      screen.getByRole("textbox", { name: "Prompt" }),
      "Explain the router",
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(
      await screen.findByText(/A feature task, so Claude · 88% sure/),
    ).toBeTruthy();
    expect(screen.queryByText(/task: feature \(0\.88\)/)).toBeNull();
    stop();
  });

  it("fills the prompt from a suggestion without sending it", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);
    render(Chat);
    await userEvent.click(
      screen.getByRole("button", { name: /Run the checks and fix what fails/ }),
    );
    const box = screen.getByRole("textbox", { name: "Prompt" });
    if (!(box instanceof HTMLTextAreaElement)) throw new Error("no textarea");
    expect(box.value).toBe("Run the checks and fix what fails");
    expect(chat.state.messages.length).toBe(0);
    chat.draft = "";
    stop();
  });
});
