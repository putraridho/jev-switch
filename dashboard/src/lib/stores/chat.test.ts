import { describe, expect, it } from "vitest";
import { fixtures } from "../ipc/fixtures";
import { createMockClient } from "../ipc/mock";
import { mergeTool, chat, initChat } from "./chat.svelte";

describe("chat store", () => {
  it("loads initial state and handles streaming events and permission replies", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);

    expect(chat.loaded).toBe(true);
    expect(chat.state.workspace).toBe("/workspace/project");

    client.pushChat({
      kind: "state",
      state: { ...chat.state, busy: true },
    });
    client.pushChat({
      kind: "route_chosen",
      route: {
        provider: "codex",
        model: "gpt-6.1-sol",
        task: "feature",
        reason: "task: feature -> codex gpt-6.1-sol",
        switched_provider: false,
      },
    });
    client.pushChat({ kind: "text_delta", text: "Hello " });
    client.pushChat({ kind: "text_delta", text: "world" });
    expect(chat.state.streaming_text).toBe("Hello world");
    expect(chat.state.active_route?.model).toBe("gpt-6.1-sol");

    client.pushChat({
      kind: "tool_activity",
      tool: {
        summary: "Bash cargo test",
        ok: null,
        output_preview: null,
        edit: null,
      },
    });
    client.pushChat({
      kind: "tool_activity",
      tool: {
        summary: "Tool result",
        ok: true,
        output_preview: "ok",
        edit: null,
      },
    });
    expect(chat.state.streaming_tools[0]?.ok).toBe(true);

    client.pushChat({
      kind: "permission_requested",
      prompt: {
        request_id: "perm_1",
        tool: "Bash",
        summary: "cargo test",
        blocked_path: null,
        can_always: true,
        always_label: "Bash(cargo test:*)",
      },
    });
    expect(chat.state.pending_permission?.request_id).toBe("perm_1");

    const replyRes = await chat.answerPermission({
      request_id: "perm_1",
      decision: "allow",
    });
    expect(replyRes.status).toBe("ok");
    expect(chat.state.pending_permission).toBeNull();

    stop();
  });

  it("updates workspace, mode, pin, orchestrate, and clears session", async () => {
    const client = createMockClient(fixtures.mixed);
    const stop = await initChat(client);

    await chat.setMode("plan");
    expect(chat.state.mode).toBe("plan");

    await chat.setPin({ kind: "provider", provider: "codex" });
    expect(chat.state.pin).toEqual({ kind: "provider", provider: "codex" });

    await chat.setOrchestrate(true);
    expect(chat.state.orchestrate).toBe(true);

    const sent = await chat.send("Fix tests");
    expect(sent.status).toBe("ok");
    expect(chat.state.messages.length).toBe(2);

    await chat.setWorkspace("/workspace/other");
    expect(chat.state.workspace).toBe("/workspace/other");
    expect(chat.state.messages.length).toBe(0);

    const dirs = await chat.listDirs(null);
    expect(dirs.status).toBe("ok");

    stop();
  });
});

describe("mergeTool", () => {
  const row = (summary: string, ok: boolean | null = null) => ({
    summary,
    ok,
    output_preview: null,
    edit: null,
  });
  const edit = { path: "a.txt", removed: [], added: ["hi"] };

  it("puts an edit on the tool row before it and waits for the result", () => {
    let tools = mergeTool([], row("Write a.txt"));
    tools = mergeTool(tools, { ...row("Tool edit"), edit });
    expect(tools).toHaveLength(1);
    expect(tools[0]?.ok).toBeNull();
    expect(tools[0]?.edit).toEqual(edit);
    tools = mergeTool(tools, row("Tool result", true));
    expect(tools).toHaveLength(1);
    expect(tools[0]?.ok).toBe(true);
  });

  it("gives an edit with no tool row its own row", () => {
    expect(mergeTool([], { ...row("Tool edit"), edit })[0]?.summary).toBe(
      "Edit a.txt",
    );
  });
});
