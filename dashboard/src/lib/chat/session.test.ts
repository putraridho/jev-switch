import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../ipc/chat-bindings/ChatMessage";
import { changedFiles, sessionTitle, tokensLine } from "./session";

function message(over: Partial<ChatMessage>): ChatMessage {
  return {
    id: 1,
    role: "assistant",
    content: "",
    provider: null,
    model: null,
    route_reason: null,
    tool_calls: [],
    thinking: null,
    changed_files: [],
    status: "answered",
    usage: null,
    attachments: [],
    ...over,
  };
}

describe("session summary", () => {
  it("names the session after its first prompt", () => {
    expect(sessionTitle([])).toBe("New session");
    expect(
      sessionTitle([
        message({ role: "user", content: "Add a dark-mode toggle." }),
      ]),
    ).toBe("Add a dark-mode toggle");
    expect(
      sessionTitle([
        message({
          role: "user",
          content:
            "Please look through the whole usage-io crate and explain how the watcher decides when to re-read",
        }),
      ]),
    ).toBe("Please look through the whole usage-io crate…");
  });

  it("prefers the title a model wrote, once the session has started", () => {
    const prompt = [
      message({
        role: "user",
        content: "hey so the cache is stale again pls fix",
      }),
    ];
    expect(sessionTitle(prompt, "Fix stale usage cache")).toBe(
      "Fix stale usage cache",
    );
    expect(sessionTitle(prompt, "   ")).toBe(
      "hey so the cache is stale again pls fix",
    );
    expect(sessionTitle(prompt, null)).toBe(
      "hey so the cache is stale again pls fix",
    );
    expect(sessionTitle([], "Left over from before")).toBe("New session");
  });

  it("collects changed files with line counts from edits", () => {
    const files = changedFiles([
      message({
        tool_calls: [
          {
            summary: "Edit a",
            ok: true,
            output_preview: null,
            edit: { path: "src/a.ts", removed: ["x"], added: ["y", "z"] },
          },
        ],
        changed_files: ["src/a.ts", "src/b.ts"],
      }),
      message({
        tool_calls: [
          {
            summary: "Edit a",
            ok: true,
            output_preview: null,
            edit: { path: "src/a.ts", removed: [], added: ["w"] },
          },
        ],
      }),
    ]);
    expect(files).toEqual([
      { path: "src/a.ts", added: 3, removed: 1 },
      { path: "src/b.ts", added: 0, removed: 0 },
    ]);
  });

  it("adds up tokens", () => {
    expect(tokensLine([])).toBe("");
    expect(
      tokensLine([
        { input_tokens: 4000, output_tokens: 600 },
        null,
        { input_tokens: 212, output_tokens: 12 },
      ]),
    ).toBe("4,212 in · 612 out");
  });
});
