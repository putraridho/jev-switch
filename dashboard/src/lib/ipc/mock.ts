// The mock `IpcClient` for unit, component and screen tests (docs/dashboard/DESIGN.md 10).

import type { CommandError } from "./bindings/CommandError";
import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { ChatEvent } from "./chat-bindings/ChatEvent";
import type { ChatSessionState } from "./chat-bindings/ChatSessionState";
import type { IpcClient } from "./client";

/** Everything a mock client answers with. */
export type MockScript = {
  snapshot: Snapshot;
  settings: Settings;
  history: HistorySeries;
  diagnostics: Diagnostics;
  chat: ChatSessionState;
  fail: Partial<
    Record<
      "refresh" | "getHistory" | "updateSettings" | "chatSend",
      CommandError
    >
  >;
};

/** One recorded command call. */
export type MockCall = {
  command:
    | "getSnapshot"
    | "refresh"
    | "getHistory"
    | "getSettings"
    | "updateSettings"
    | "getDiagnostics"
    | "chatGetState"
    | "chatSend"
    | "chatCancel"
    | "chatSetWorkspace"
    | "chatSetMode"
    | "chatSetPin"
    | "chatSetOrchestrate"
    | "chatAnswerPermission"
    | "chatNewSession"
    | "chatListDirs";
};

/** A client a test can drive: push snapshots, chat events, send routes, read the calls made. */
export interface MockIpcClient extends IpcClient {
  push(s: Snapshot): void;
  pushChat(e: ChatEvent): void;
  navigate(r: Route): void;
  readonly calls: readonly MockCall[];
}

/** A mock client that answers from `script`. */
export function createMockClient(script: MockScript): MockIpcClient {
  const calls: MockCall[] = [];
  const snapshots = new Set<(snapshot: Snapshot) => void>();
  const routes = new Set<(route: Route) => void>();
  const chatEvents = new Set<(event: ChatEvent) => void>();
  let currentSnapshot = script.snapshot;
  let currentSettings = { ...script.settings };
  let currentChat: ChatSessionState = {
    ...script.chat,
    messages: [...script.chat.messages],
    providers: [...script.chat.providers],
    models: [...script.chat.models],
  };

  function emitChat(event: ChatEvent): void {
    chatEvents.forEach((cb) => {
      cb(event);
    });
  }

  return {
    getSnapshot: () => {
      calls.push({ command: "getSnapshot" });
      return Promise.resolve({ status: "ok", value: currentSnapshot });
    },
    refresh: () => {
      calls.push({ command: "refresh" });
      const error = script.fail.refresh;
      return Promise.resolve(
        error ? { status: "error", error } : { status: "ok", value: null },
      );
    },
    getHistory: () => {
      calls.push({ command: "getHistory" });
      const error = script.fail.getHistory;
      return Promise.resolve(
        error
          ? { status: "error", error }
          : { status: "ok", value: script.history },
      );
    },
    getSettings: () => {
      calls.push({ command: "getSettings" });
      return Promise.resolve({ status: "ok", value: currentSettings });
    },
    updateSettings: (patch) => {
      calls.push({ command: "updateSettings" });
      const error = script.fail.updateSettings;
      if (error) return Promise.resolve({ status: "error", error });
      currentSettings = { ...currentSettings, ...patch };
      return Promise.resolve({ status: "ok", value: currentSettings });
    },
    getDiagnostics: () => {
      calls.push({ command: "getDiagnostics" });
      return Promise.resolve({ status: "ok", value: script.diagnostics });
    },
    onSnapshot: (cb) => {
      snapshots.add(cb);
      return Promise.resolve(() => {
        snapshots.delete(cb);
      });
    },
    onNavigate: (cb) => {
      routes.add(cb);
      return Promise.resolve(() => {
        routes.delete(cb);
      });
    },

    chatGetState: () => {
      calls.push({ command: "chatGetState" });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatSend: (prompt) => {
      calls.push({ command: "chatSend" });
      const error = script.fail.chatSend;
      if (error) return Promise.resolve({ status: "error", error });
      const trimmed = prompt.trim();
      if (trimmed.length === 0) {
        return Promise.resolve({
          status: "error",
          error: { code: "invalid_argument" },
        });
      }
      const nextId = currentChat.messages.length + 1;
      currentChat = {
        ...currentChat,
        messages: [
          ...currentChat.messages,
          {
            id: nextId,
            role: "user",
            content: trimmed,
            provider: null,
            model: null,
            route_reason: null,
            tool_calls: [],
            thinking: null,
            changed_files: [],
            status: "answered",
            usage: null,
          },
          {
            id: nextId + 1,
            role: "assistant",
            content: `Mock reply in ${currentChat.workspace} (${currentChat.mode} mode).`,
            provider: "claude",
            model: "claude-sonnet-5-5",
            route_reason: "task: feature (0.88) -> claude claude-sonnet-5-5",
            tool_calls: [],
            thinking: null,
            changed_files: [],
            status: "answered",
            usage: {
              input_tokens: 420,
              output_tokens: 64,
              cache_read_tokens: 0,
            },
          },
        ],
      };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatCancel: () => {
      calls.push({ command: "chatCancel" });
      currentChat = { ...currentChat, busy: false, pending_permission: null };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatSetWorkspace: (path) => {
      calls.push({ command: "chatSetWorkspace" });
      currentChat = {
        ...currentChat,
        workspace: path,
        messages: [],
        notes: [],
        active_route: null,
      };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatSetMode: (mode) => {
      calls.push({ command: "chatSetMode" });
      currentChat = { ...currentChat, mode };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatSetPin: (pin) => {
      calls.push({ command: "chatSetPin" });
      currentChat = { ...currentChat, pin };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatSetOrchestrate: (on) => {
      calls.push({ command: "chatSetOrchestrate" });
      currentChat = { ...currentChat, orchestrate: on };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatAnswerPermission: () => {
      calls.push({ command: "chatAnswerPermission" });
      currentChat = { ...currentChat, pending_permission: null };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: null });
    },
    chatNewSession: () => {
      calls.push({ command: "chatNewSession" });
      currentChat = {
        ...currentChat,
        messages: [],
        notes: [],
        streaming_text: "",
        streaming_thinking: "",
        streaming_tools: [],
        active_route: null,
        pending_permission: null,
      };
      emitChat({ kind: "state", state: currentChat });
      return Promise.resolve({ status: "ok", value: currentChat });
    },
    chatListDirs: (path) => {
      calls.push({ command: "chatListDirs" });
      const base = path ?? currentChat.workspace;
      return Promise.resolve({
        status: "ok",
        value: {
          current: base,
          parent: base === "/" ? null : "/workspace",
          dirs: [`${base}/crates`, `${base}/dashboard`, `${base}/docs`],
        },
      });
    },
    onChatEvent: (cb) => {
      chatEvents.add(cb);
      return Promise.resolve(() => {
        chatEvents.delete(cb);
      });
    },

    push: (snapshot) => {
      currentSnapshot = snapshot;
      snapshots.forEach((cb) => {
        cb(snapshot);
      });
    },
    pushChat: (event) => {
      if (event.kind === "state") currentChat = event.state;
      emitChat(event);
    },
    navigate: (route) => {
      routes.forEach((cb) => {
        cb(route);
      });
    },
    calls,
  };
}
