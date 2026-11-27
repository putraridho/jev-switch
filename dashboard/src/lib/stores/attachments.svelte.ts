// The composer's tray: files waiting to go with the next message. Each one is read, handed
// to the app to keep, and shown as a chip that's reading, ready or refused (with why).
// Like the draft, the tray outlives the Chat screen.

import { SvelteMap } from "svelte/reactivity";
import {
  MAX_BYTES,
  MAX_PER_MESSAGE,
  formatBytes,
  isImageType,
  toBase64,
} from "../chat/attachments";
import type { ChatAttachment } from "../ipc/chat-bindings/ChatAttachment";
import type { ChatAttachResult } from "../ipc/chat-bindings/ChatAttachResult";
import type { Outcome } from "../ipc/bindings/Outcome";
import type { IpcClient } from "../ipc/client";
import { announce } from "./announce.svelte";

/** One chip in the tray. */
export type TrayItem =
  | { id: number; status: "reading"; name: string; bytes: number | null }
  | { id: number; status: "ready"; attachment: ChatAttachment }
  | {
      id: number;
      status: "refused";
      name: string;
      bytes: number | null;
      reason: string;
    };

let items = $state<TrayItem[]>([]);
let nextId = 1;
let client: IpcClient | null = null;

/** Small previews of pictures, by staged path, kept for the session so sent messages show
    them too. Drawn on a canvas: the app's CSP allows no blob: or data: images. */
export const previews = new SvelteMap<string, ImageBitmap>();

/** The preview height in CSS pixels at 2x, the largest place a preview is drawn. */
const PREVIEW_H = 208;

function replace(id: number, item: TrayItem): void {
  items = items.map((it) => (it.id === id ? item : it));
}

function settle(
  id: number,
  name: string,
  bytes: number | null,
  result: Outcome<ChatAttachResult>,
): ChatAttachment | null {
  if (!items.some((it) => it.id === id)) return null; // removed while reading
  if (result.status === "error") {
    replace(id, {
      id,
      status: "refused",
      name,
      bytes,
      reason: "couldn't attach",
    });
    return null;
  }
  const value = result.value;
  if (value.status === "refused") {
    replace(id, {
      id,
      status: "refused",
      name: value.name,
      bytes: value.bytes,
      reason: value.reason,
    });
    announce.say(`${value.name} can't be attached: ${value.reason}`);
    return null;
  }
  replace(id, { id, status: "ready", attachment: value.attachment });
  return value.attachment;
}

function room(): number {
  return MAX_PER_MESSAGE - items.length;
}

async function addFile(file: File): Promise<void> {
  const id = nextId++;
  if (file.size > MAX_BYTES) {
    items = [
      ...items,
      {
        id,
        status: "refused",
        name: file.name,
        bytes: file.size,
        reason: "over 20 MB",
      },
    ];
    announce.say(`${file.name} is over 20 MB`);
    return;
  }
  items = [
    ...items,
    { id, status: "reading", name: file.name, bytes: file.size },
  ];
  const ipc = client;
  if (ipc === null) return;
  const preview =
    isImageType(file.type) && typeof createImageBitmap === "function"
      ? createImageBitmap(file, { resizeHeight: PREVIEW_H }).catch(() => null)
      : Promise.resolve(null);
  const data = toBase64(new Uint8Array(await file.arrayBuffer()));
  const kept = settle(
    id,
    file.name,
    file.size,
    await ipc.chatAttachData(file.name, data),
  );
  const bitmap = await preview;
  if (kept !== null && bitmap !== null && kept.kind === "image") {
    previews.set(kept.path, bitmap);
  }
}

export const tray = {
  /** Connects the tray to the app. */
  init(ipc: IpcClient): void {
    client = ipc;
  },
  get items(): readonly TrayItem[] {
    return items;
  },
  /** Every chip is ready (none reading, none refused) and there's at least one. */
  get ready(): boolean {
    return items.length > 0 && items.every((it) => it.status === "ready");
  },
  /** Nothing in the tray stops a send (an empty tray doesn't). */
  get clear(): boolean {
    return items.every((it) => it.status === "ready");
  },
  get hasImage(): boolean {
    return items.some(
      (it) => it.status === "ready" && it.attachment.kind === "image",
    );
  },
  /** The kept attachments, in order. */
  get attachments(): ChatAttachment[] {
    return items.flatMap((it) =>
      it.status === "ready" ? [it.attachment] : [],
    );
  },
  /** Adds dropped, picked or pasted files; past ten, the rest are refused. */
  async addFiles(files: readonly File[]): Promise<void> {
    const fits = files.slice(0, Math.max(0, room()));
    for (const extra of files.slice(fits.length)) {
      items = [
        ...items,
        {
          id: nextId++,
          status: "refused",
          name: extra.name,
          bytes: extra.size,
          reason: `${String(MAX_PER_MESSAGE)} at a time`,
        },
      ];
    }
    await Promise.all(fits.map((f) => addFile(f)));
  },
  /** Adds long pasted text as an attachment. */
  async addText(text: string): Promise<void> {
    const ipc = client;
    if (ipc === null || room() <= 0) return;
    const id = nextId++;
    items = [
      ...items,
      { id, status: "reading", name: "Pasted text", bytes: text.length },
    ];
    settle(id, "Pasted text", text.length, await ipc.chatAttachText(text));
  },
  remove(id: number): void {
    const gone = items.find((it) => it.id === id);
    items = items.filter((it) => it.id !== id);
    if (gone) {
      announce.say(
        `Removed ${gone.status === "ready" ? gone.attachment.name : gone.name}`,
      );
    }
  },
  /** Empties the tray after a send; previews stay for the transcript. */
  clearAll(): void {
    items = [];
  },
  /** Puts sent attachments back after a send fails. */
  restore(attachments: readonly ChatAttachment[]): void {
    items = attachments.map((attachment) => ({
      id: nextId++,
      status: "ready",
      attachment,
    }));
  },
};

/** The second line of a chip that isn't ready: "Reading · 2.4 MB", "212 MB · over 20 MB". */
export function pendingMeta(item: TrayItem): string {
  if (item.status === "reading") {
    return item.bytes === null
      ? "Reading"
      : `Reading · ${formatBytes(item.bytes)}`;
  }
  if (item.status === "refused") {
    return item.bytes === null
      ? item.reason
      : `${formatBytes(item.bytes)} · ${item.reason}`;
  }
  return "";
}
