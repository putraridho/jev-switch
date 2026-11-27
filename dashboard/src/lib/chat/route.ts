// The router's note on a turn, for people. The engine writes notes like
// "task: feature (0.88) -> claude claude-sonnet-5-5"; the chat shows
// "A feature task, so Claude · 88% sure". Anything else is shown as written.
import { providerName, providerOrder } from "../copy";
import type { ProviderKey } from "../ipc/bindings/ProviderKey";

export interface RouteNote {
  task: string;
  /** 0..1 */
  confidence: number;
  provider: ProviderKey;
  model: string;
}

const NOTE = /^task:\s*([\w-]+)\s*\(([\d.]+)\)\s*->\s*([\w-]+)\s+(\S+)\s*$/;

export function parseRouteNote(note: string): RouteNote | null {
  const hit = NOTE.exec(note.trim());
  if (hit === null) return null;
  const task: string = hit[1] ?? "";
  const conf: string = hit[2] ?? "";
  const prov: string = hit[3] ?? "";
  const model: string = hit[4] ?? "";
  const provider = providerOrder.find((key) => key === prov);
  const confidence = Number(conf);
  if (provider === undefined || !Number.isFinite(confidence)) return null;
  return { task: task.replaceAll("_", " "), confidence, provider, model };
}

/** "A feature task, so Claude · 88% sure"; the note itself when it isn't a routing note. */
export function routeSentence(note: string): string {
  const parsed = parseRouteNote(note);
  if (parsed === null) return note;
  const article = /^[aeiou]/i.test(parsed.task) ? "An" : "A";
  const sure = Math.round(Math.min(1, Math.max(0, parsed.confidence)) * 100);
  return `${article} ${parsed.task} task, so ${providerName[parsed.provider]} · ${String(sure)}% sure`;
}
