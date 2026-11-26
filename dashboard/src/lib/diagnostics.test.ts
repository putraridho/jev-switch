import { describe, expect, it } from "vitest";
import {
  bytesLabel,
  countLabel,
  errorWord,
  realSources,
  sourceName,
  sourceStatus,
} from "./diagnostics";
import { FIXTURE_NOW_MS, fixtures } from "./ipc/fixtures";
import type { SourceDiag } from "./ipc/bindings/SourceDiag";
import type { Unavailable } from "./ipc/bindings/Unavailable";

const sources = fixtures.known.diagnostics.sources;

describe("diagnostics words", () => {
  it("names every source and hides the placeholders", () => {
    expect(sourceName("claude_gateway_log")).toBe("Claude gateway log");
    expect(realSources(sources).map((item) => item.source)).toEqual([
      "claude_gateway_log",
      "codex_app_server",
      "deepseek_balance",
    ]);
  });

  it("has words for every kind of failure", () => {
    const kinds: Unavailable[] = [
      "not_installed",
      "not_logged_in",
      "offline",
      "timeout",
      "rate_limited",
      "unauthorized",
      "server_error",
      "protocol",
      "io",
      "internal",
    ];
    for (const kind of kinds) expect(errorWord(kind)).not.toBe("");
  });

  it("states the last success, the last problem and the streak", () => {
    const ok = sources[0];
    const failing = sources[1];
    const never = sources[2];
    if (ok === undefined || failing === undefined || never === undefined)
      throw new Error("fixture sources missing");
    expect(sourceStatus(ok, "h24")).toMatch(/^ok \d\d:\d\d$/);
    expect(sourceStatus(failing, "h24")).toMatch(
      /^ok \d\d:\d\d · last problem: offline · 2 failed in a row$/,
    );
    const none: SourceDiag = { ...never, last_ok_at: null };
    expect(sourceStatus(none, "h24")).toBe("no success yet");
    expect(FIXTURE_NOW_MS).toBeGreaterThan(0);
  });

  it("sizes the history file and counts samples", () => {
    expect(bytesLabel(0)).toBe("0 B");
    expect(bytesLabel(840)).toBe("840 B");
    expect(bytesLabel(1_240_000)).toBe("1.2 MB");
    expect(bytesLabel(52_000_000)).toBe("52 MB");
    expect(bytesLabel(3_000_000_000_000)).toBe("3000 GB");
    expect(countLabel(8412)).toBe("8\u202f412");
    expect(countLabel(12)).toBe("12");
  });
});
