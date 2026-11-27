// What the chat says about the session as a whole: its title, the files it changed and the
// tokens it used. Pure functions of the transcript.
import type { ChatMessage } from "../ipc/chat-bindings/ChatMessage";

export interface ChangedFile {
  path: string;
  added: number;
  removed: number;
}

/**
 * The session's name: the title a model wrote after the first reply when there is one,
 * else (until it arrives, or when none could be written) the first prompt, cut at a word
 * near 48 characters.
 */
export function sessionTitle(
  messages: readonly ChatMessage[],
  generated: string | null = null,
): string {
  const written = generated?.trim() ?? "";
  if (written !== "" && messages.length > 0) return written;
  const first = messages.find((m) => m.role === "user")?.content.trim() ?? "";
  if (first === "") return "New session";
  const line = first.split("\n")[0] ?? first;
  if (line.length <= 48) return line.replace(/[.?!]+$/, "");
  const cut = line.slice(0, 48);
  const space = cut.lastIndexOf(" ");
  return `${(space > 24 ? cut.slice(0, space) : cut).replace(/[,;:]$/, "")}…`;
}

/** Every file the session changed, with line counts from the edits it saw. */
export function changedFiles(messages: readonly ChatMessage[]): ChangedFile[] {
  const files = new Map<string, ChangedFile>();
  const touch = (path: string): ChangedFile => {
    const known = files.get(path);
    if (known) return known;
    const fresh = { path, added: 0, removed: 0 };
    files.set(path, fresh);
    return fresh;
  };
  for (const message of messages) {
    for (const tool of message.tool_calls) {
      if (tool.edit === null) continue;
      const file = touch(tool.edit.path);
      file.added += tool.edit.added.length;
      file.removed += tool.edit.removed.length;
    }
    for (const path of message.changed_files) touch(path);
  }
  return [...files.values()];
}

const count = new Intl.NumberFormat("en-US");

/** "4,212 in · 612 out" for one turn's usage, or a whole session's. */
export function tokensLine(
  usages: readonly ({ input_tokens: number; output_tokens: number } | null)[],
): string {
  let input = 0;
  let output = 0;
  let any = false;
  for (const u of usages) {
    if (u === null) continue;
    any = true;
    input += u.input_tokens;
    output += u.output_tokens;
  }
  return any ? `${count.format(input)} in · ${count.format(output)} out` : "";
}
