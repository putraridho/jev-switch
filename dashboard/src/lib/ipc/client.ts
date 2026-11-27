// The IPC seam (docs/dashboard/DESIGN.md 7.3, 10). Components and stores import ONLY this
// interface, never `@tauri-apps/api`. Every type is generated from Rust; nothing here is
// hand-typed twice.

import type { Diagnostics } from "./bindings/Diagnostics";
import type { HistoryQuery } from "./bindings/HistoryQuery";
import type { HistorySeries } from "./bindings/HistorySeries";
import type { Outcome } from "./bindings/Outcome";
import type { RefreshTarget } from "./bindings/RefreshTarget";
import type { Route } from "./bindings/Route";
import type { Settings } from "./bindings/Settings";
import type { SettingsPatch } from "./bindings/SettingsPatch";
import type { Snapshot } from "./bindings/Snapshot";
import type { ChatAttachResult } from "./chat-bindings/ChatAttachResult";
import type { ChatDirListing } from "./chat-bindings/ChatDirListing";
import type { ChatEvent } from "./chat-bindings/ChatEvent";
import type { ChatMode } from "./chat-bindings/ChatMode";
import type { ChatPermissionReply } from "./chat-bindings/ChatPermissionReply";
import type { ChatPin } from "./chat-bindings/ChatPin";
import type { ChatSessionState } from "./chat-bindings/ChatSessionState";

/** The usage and coding-agent chat commands and events. Every command resolves with `Outcome`. */
export interface IpcClient {
  getSnapshot(): Promise<Outcome<Snapshot>>;
  refresh(target: RefreshTarget): Promise<Outcome<null>>;
  getHistory(query: HistoryQuery): Promise<Outcome<HistorySeries>>;
  getSettings(): Promise<Outcome<Settings>>;
  updateSettings(patch: SettingsPatch): Promise<Outcome<Settings>>;
  getDiagnostics(): Promise<Outcome<Diagnostics>>;
  onSnapshot(cb: (s: Snapshot) => void): Promise<() => void>;
  onNavigate(cb: (r: Route) => void): Promise<() => void>;

  chatGetState(): Promise<Outcome<ChatSessionState>>;
  /** Sends `prompt` with the attachments at `attachments` (paths from the attach commands). */
  chatSend(
    prompt: string,
    attachments: readonly string[],
  ): Promise<Outcome<ChatSessionState>>;
  /** Keeps a dropped, picked or pasted file (`data` in base64) as an attachment. */
  chatAttachData(
    name: string,
    data: string,
  ): Promise<Outcome<ChatAttachResult>>;
  /** Keeps long pasted text as an attachment. */
  chatAttachText(text: string): Promise<Outcome<ChatAttachResult>>;
  chatCancel(): Promise<Outcome<ChatSessionState>>;
  chatSetWorkspace(path: string): Promise<Outcome<ChatSessionState>>;
  chatSetMode(mode: ChatMode): Promise<Outcome<ChatSessionState>>;
  chatSetPin(pin: ChatPin): Promise<Outcome<ChatSessionState>>;
  chatSetOrchestrate(on: boolean): Promise<Outcome<ChatSessionState>>;
  chatAnswerPermission(reply: ChatPermissionReply): Promise<Outcome<null>>;
  chatNewSession(): Promise<Outcome<ChatSessionState>>;
  chatListDirs(path: string | null): Promise<Outcome<ChatDirListing>>;
  onChatEvent(cb: (e: ChatEvent) => void): Promise<() => void>;
}
