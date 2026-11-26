// The words of "About and diagnostics" (DESIGN 9.6): typed values in, short sentences out.
// Nothing here can print an error message; every value is a number or a closed enum.

import { timeOfDay } from "./format";
import type { HourCycle } from "./ipc/bindings/HourCycle";
import type { SourceDiag } from "./ipc/bindings/SourceDiag";
import type { SourceId } from "./ipc/bindings/SourceId";
import type { Unavailable } from "./ipc/bindings/Unavailable";

const sourceNames: Record<SourceId, string> = {
  claude_gateway_log: "Claude gateway log",
  claude_live: "Claude live check",
  codex_app_server: "Codex app-server",
  deepseek_balance: "DeepSeek balance",
  gemini_none: "Gemini (no source yet)",
  antigravity_cli: "Antigravity quota",
  grok_none: "Grok (no source yet)",
};

/** The display name of a source. */
export function sourceName(source: SourceId): string {
  return sourceNames[source];
}

/** Sources that read something; the placeholders are listed in Settings, Providers. */
export function realSources(sources: readonly SourceDiag[]): SourceDiag[] {
  return sources.filter((item) => !item.source.endsWith("_none"));
}

const errorWords: Record<Unavailable, string> = {
  not_installed: "not installed",
  not_logged_in: "not signed in",
  offline: "offline",
  timeout: "timed out",
  rate_limited: "rate limited",
  unauthorized: "credentials not accepted",
  server_error: "server error",
  protocol: "unexpected answer",
  io: "could not read data",
  internal: "internal error",
};

/** The short words for the kind of the last failure. */
export function errorWord(reason: Unavailable): string {
  return errorWords[reason];
}

/** "ok 12:33", or "not yet", then the last failure and the failures in a row. */
export function sourceStatus(item: SourceDiag, hour: HourCycle): string {
  const parts = [
    item.last_ok_at === null
      ? "no success yet"
      : `ok ${timeOfDay(item.last_ok_at, hour)}`,
  ];
  if (item.last_error !== null) {
    parts.push(`last problem: ${errorWord(item.last_error)}`);
  }
  if (item.failures > 0) {
    parts.push(`${String(item.failures)} failed in a row`);
  }
  return parts.join(" · ");
}

/** "0 B", "840 B", "1.2 MB": the history file's size. */
export function bytesLabel(bytes: number): string {
  if (bytes < 1_000) return `${String(Math.max(0, Math.round(bytes)))} B`;
  const units = ["kB", "MB", "GB"];
  let value = bytes / 1_000;
  let unit = 0;
  while (value >= 1_000 && unit < units.length - 1) {
    value /= 1_000;
    unit += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit] ?? "GB"}`;
}

/** "8 412": a count with a thin space between thousands, like the wireframe. */
export function countLabel(n: number): string {
  return String(Math.round(n)).replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}
