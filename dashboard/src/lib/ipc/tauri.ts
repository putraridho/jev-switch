import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Outcome } from "./bindings/Outcome";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { Snapshot } from "./bindings/Snapshot";
import type { ChatAttachResult } from "./chat-bindings/ChatAttachResult";
import type { ChatDirListing } from "./chat-bindings/ChatDirListing";
import type { ChatEvent } from "./chat-bindings/ChatEvent";
import type { ChatSessionState } from "./chat-bindings/ChatSessionState";
import type { IpcClient } from "./client";

/** The client the real app uses. */
export function createTauriClient(): IpcClient {
  return {
    getSnapshot: () =>
      invoke<Outcome<Snapshot>>("get_snapshot").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    refresh: (target) =>
      invoke<Outcome<null>>("refresh", { target }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getHistory: (query) =>
      invoke<Outcome<HistorySeries>>("get_history", { query }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getSettings: () =>
      invoke<Outcome<Settings>>("get_settings").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    updateSettings: (patch) =>
      invoke<Outcome<Settings>>("update_settings", { patch }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    getDiagnostics: () =>
      invoke<Outcome<Diagnostics>>("get_diagnostics").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    onSnapshot: (cb) =>
      listen<Snapshot>("snapshot", (event) => {
        cb(event.payload);
      }),
    onNavigate: (cb) =>
      listen<Route>("navigate", (event) => {
        cb(event.payload);
      }),

    chatGetState: () =>
      invoke<Outcome<ChatSessionState>>("chat_get_state").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatSend: (prompt, attachments) =>
      invoke<Outcome<ChatSessionState>>("chat_send", {
        prompt,
        attachments,
      }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatAttachData: (name, data) =>
      invoke<Outcome<ChatAttachResult>>("chat_attach_data", {
        name,
        data,
      }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatAttachText: (text) =>
      invoke<Outcome<ChatAttachResult>>("chat_attach_text", { text }).catch(
        () => ({
          status: "error",
          error: { code: "internal" },
        }),
      ),
    chatCancel: () =>
      invoke<Outcome<ChatSessionState>>("chat_cancel").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatSetWorkspace: (path) =>
      invoke<Outcome<ChatSessionState>>("chat_set_workspace", { path }).catch(
        () => ({
          status: "error",
          error: { code: "internal" },
        }),
      ),
    chatSetMode: (mode) =>
      invoke<Outcome<ChatSessionState>>("chat_set_mode", { mode }).catch(
        () => ({
          status: "error",
          error: { code: "internal" },
        }),
      ),
    chatSetPin: (pin) =>
      invoke<Outcome<ChatSessionState>>("chat_set_pin", { pin }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatSetOrchestrate: (on) =>
      invoke<Outcome<ChatSessionState>>("chat_set_orchestrate", { on }).catch(
        () => ({
          status: "error",
          error: { code: "internal" },
        }),
      ),
    chatAnswerPermission: (reply) =>
      invoke<Outcome<null>>("chat_answer_permission", { reply }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatNewSession: () =>
      invoke<Outcome<ChatSessionState>>("chat_new_session").catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    chatListDirs: (path) =>
      invoke<Outcome<ChatDirListing>>("chat_list_dirs", { path }).catch(() => ({
        status: "error",
        error: { code: "internal" },
      })),
    onChatEvent: (cb) =>
      listen<ChatEvent>("chat_event", (event) => {
        cb(event.payload);
      }),
  };
}
