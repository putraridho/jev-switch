// Attachment rules the composer applies before anything reaches the app: the limits, the
// words on a chip, when a paste becomes an attachment, and base64 for the trip over IPC.
// The app checks every file again (dashboard/crates/chat-io/src/attachments.rs).

import type { ChatAttachment } from "../ipc/chat-bindings/ChatAttachment";

/** The largest file that can be attached (20 MB), the same limit as the app's. */
export const MAX_BYTES = 20 * 1024 * 1024;

/** The most attachments one message carries. */
export const MAX_PER_MESSAGE = 10;

/** A paste this long goes in as an attachment, so the prompt stays readable. */
export const LONG_PASTE_LINES = 40;
export const LONG_PASTE_CHARS = 2000;

/** What is accepted, for the file picker and the drop hint. */
export const ACCEPT =
  "image/png,image/jpeg,image/gif,image/webp,application/pdf,text/*,.md,.json,.ts,.js,.svelte,.rs,.py,.go,.toml,.yaml,.yml,.css,.html,.sh,.sql,.csv,.log";

/** "4 KB", "1.2 MB", "212 MB": one decimal only below 10. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${String(Math.max(1, Math.round(kb)))} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : String(Math.round(mb))} MB`;
}

/** The type tag on a chip: the extension in capitals ("CSS", "PDF"), at most four letters. */
export function typeTag(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "FILE";
  return name
    .slice(dot + 1)
    .slice(0, 4)
    .toUpperCase();
}

/** The second line of a ready chip: "84 lines" for pasted text, the size otherwise. */
export function chipMeta(a: ChatAttachment): string {
  if (a.kind === "text" && a.lines !== null) {
    return a.lines === 1 ? "1 line" : `${String(a.lines)} lines`;
  }
  return formatBytes(a.bytes);
}

/** Whether pasted text is long enough to go in as an attachment rather than inline. */
export function isLongPaste(text: string): boolean {
  if (text.length >= LONG_PASTE_CHARS) return true;
  let lines = 1;
  for (const ch of text) {
    if (ch === "\n") lines += 1;
    if (lines > LONG_PASTE_LINES) return true;
  }
  return false;
}

/** Whether a file looks like a picture the composer can preview. */
export function isImageType(type: string): boolean {
  return /^image\/(png|jpeg|gif|webp)$/.test(type);
}

/** Base64 of `bytes`, in chunks so a 20 MB file never builds one huge argument list. */
export function toBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/** Whether a drag carries files (and not text or a link being dragged around). */
export function dragHasFiles(types: readonly string[]): boolean {
  return types.includes("Files");
}
