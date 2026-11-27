import type { Outcome } from "../ipc/bindings/Outcome";
import type { ChatDirListing } from "../ipc/chat-bindings/ChatDirListing";
import type { ChatEvent } from "../ipc/chat-bindings/ChatEvent";
import type { ChatMode } from "../ipc/chat-bindings/ChatMode";
import type { ChatPermissionReply } from "../ipc/chat-bindings/ChatPermissionReply";
import type { ChatPin } from "../ipc/chat-bindings/ChatPin";
import type { ChatSessionState } from "../ipc/chat-bindings/ChatSessionState";
import type { ChatToolCall } from "../ipc/chat-bindings/ChatToolCall";
import type { IpcClient } from "../ipc/client";
import { announce } from "./announce.svelte";

const emptyState: ChatSessionState = {
  workspace: "",
  mode: "edit",
  pin: { kind: "auto" },
  routing_enabled: true,
  orchestrate: false,
  jev_key_present: false,
  busy: false,
  active_route: null,
  streaming_text: "",
  streaming_thinking: "",
  streaming_tools: [],
  notes: [],
  pending_permission: null,
  messages: [],
  providers: [],
  models: [],
};

let state = $state<ChatSessionState>(emptyState);
let loaded = $state(false);
// The unsent prompt outlives the Chat screen, so leaving the tab (or Esc) never loses it.
let draft = $state("");
let client: IpcClient | null = null;

/** The summaries that carry the result or the edit diff of the tool row before them. */
const TOOL_RESULT = "Tool result";
const TOOL_EDIT = "Tool edit";

/** Folds one tool event into the list, the same way the Rust service does. */
export function mergeTool(
  tools: readonly ChatToolCall[],
  tool: ChatToolCall,
): ChatToolCall[] {
  const last = tools[tools.length - 1];
  if (tool.summary === TOOL_RESULT && last !== undefined) {
    return [
      ...tools.slice(0, -1),
      { ...last, ok: tool.ok, output_preview: tool.output_preview },
    ];
  }
  if (tool.summary === TOOL_EDIT) {
    if (last !== undefined) {
      return [...tools.slice(0, -1), { ...last, edit: tool.edit }];
    }
    return [
      { ...tool, summary: tool.edit ? `Edit ${tool.edit.path}` : "Edit" },
    ];
  }
  return [...tools, tool];
}

function applyEvent(event: ChatEvent): void {
  switch (event.kind) {
    case "state": {
      const wasBusy = state.busy;
      state = event.state;
      loaded = true;
      if (wasBusy && !event.state.busy) {
        const last = event.state.messages[event.state.messages.length - 1];
        if (last?.role === "assistant") {
          announce.say(
            last.status === "cancelled"
              ? "Turn stopped"
              : last.status === "answered"
                ? "Reply ready"
                : "Turn ended",
          );
        }
      }
      return;
    }
    case "text_delta":
      state = { ...state, streaming_text: state.streaming_text + event.text };
      return;
    case "thinking_delta":
      state = {
        ...state,
        streaming_thinking: state.streaming_thinking + event.text,
      };
      return;
    case "route_chosen":
      state = { ...state, active_route: event.route };
      return;
    case "tool_activity":
      state = {
        ...state,
        streaming_tools: mergeTool(state.streaming_tools, event.tool),
      };
      return;
    case "note":
      state = { ...state, notes: [...state.notes, event.line] };
      return;
    case "permission_requested":
      state = { ...state, pending_permission: event.prompt };
      announce.say(`Approval requested for ${event.prompt.tool}`);
      return;
    case "permission_cleared":
      if (state.pending_permission?.request_id === event.request_id) {
        state = { ...state, pending_permission: null };
      }
      return;
  }
}

/** Subscribes to live chat events and fetches the initial coding-agent state. */
export async function initChat(ipc: IpcClient): Promise<() => void> {
  client = ipc;
  loaded = false;
  state = emptyState;
  let active = true;
  const unsubscribe = await ipc.onChatEvent((event) => {
    if (active) applyEvent(event);
  });
  const result = await ipc.chatGetState();
  if (result.status === "ok") {
    state = result.value;
    loaded = true;
  }
  return () => {
    active = false;
    unsubscribe();
  };
}

export const chat = {
  get state(): ChatSessionState {
    return state;
  },
  get loaded(): boolean {
    return loaded;
  },
  get draft(): string {
    return draft;
  },
  set draft(value: string) {
    draft = value;
  },
  async send(prompt: string): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatSend(prompt);
    if (result.status === "ok") {
      state = result.value;
      loaded = true;
    }
    return result;
  },
  async cancel(): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatCancel();
    if (result.status === "ok") state = result.value;
    return result;
  },
  async setWorkspace(path: string): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatSetWorkspace(path);
    if (result.status === "ok") {
      state = result.value;
      announce.say("Workspace changed");
    }
    return result;
  },
  async setMode(mode: ChatMode): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatSetMode(mode);
    if (result.status === "ok") state = result.value;
    return result;
  },
  async setPin(pin: ChatPin): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatSetPin(pin);
    if (result.status === "ok") state = result.value;
    return result;
  },
  async setOrchestrate(on: boolean): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatSetOrchestrate(on);
    if (result.status === "ok") state = result.value;
    return result;
  },
  async answerPermission(reply: ChatPermissionReply): Promise<Outcome<null>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatAnswerPermission(reply);
    if (result.status === "ok") {
      state = { ...state, pending_permission: null };
    }
    return result;
  },
  async newSession(): Promise<Outcome<ChatSessionState>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    const result = await client.chatNewSession();
    if (result.status === "ok") {
      state = result.value;
      announce.say("New conversation started");
    }
    return result;
  },
  async listDirs(path: string | null): Promise<Outcome<ChatDirListing>> {
    if (client === null)
      return { status: "error", error: { code: "internal" } };
    return client.chatListDirs(path);
  },
};
