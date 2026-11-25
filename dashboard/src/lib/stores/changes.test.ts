import { describe, expect, it } from "vitest";
import { fixtures } from "../ipc/fixtures";
import type { Snapshot } from "../ipc/bindings/Snapshot";
import { snapshotChanges } from "./changes";

const now = 1_797_226_380_000;
const known = fixtures.known.snapshot;
const warn = fixtures.warn.snapshot;
const critical = fixtures.critical.snapshot;

describe("snapshotChanges", () => {
  it("announces nothing for the first snapshot", () => {
    expect(snapshotChanges(null, warn, now, "h24")).toEqual([]);
  });

  it("announces a meter that crosses into warn", () => {
    const sentences = snapshotChanges(known, warn, now, "h24");
    expect(sentences.length).toBe(1);
    expect(sentences[0]).toMatch(/^Claude 5-hour at 86%\. Resets /);
  });

  it("announces warn to critical and back to ok", () => {
    expect(snapshotChanges(warn, critical, now, "h24")[0]).toMatch(
      /^Claude 5-hour at 97%/,
    );
    expect(snapshotChanges(warn, known, now, "h24")[0]).toMatch(
      /^Claude 5-hour is back to normal at 50%/,
    );
  });

  it("never announces a number that moves within a level", () => {
    const moved: Snapshot = {
      ...known,
      providers: known.providers.map((p) =>
        p.usage.state === "known" && p.provider === "claude"
          ? {
              ...p,
              usage: {
                ...p.usage,
                reading: {
                  ...p.usage.reading,
                  meters: p.usage.reading.meters.map((m) =>
                    m.kind === "window"
                      ? { ...m, used_bp: m.used_bp + 300 }
                      : m,
                  ),
                },
              },
            }
          : p,
      ),
    };
    expect(snapshotChanges(known, moved, now, "h24")).toEqual([]);
  });

  it("announces a provider whose state changes", () => {
    const sentences = snapshotChanges(
      known,
      fixtures.stale_failing.snapshot,
      now,
      "h24",
    );
    expect(sentences).toEqual(["Can't reach Codex. Showing the last reading."]);
  });

  it("says nothing about hidden providers or ones that just appeared", () => {
    const hidden: Snapshot = {
      ...warn,
      providers: warn.providers.map((p) => ({ ...p, enabled: false })),
    };
    expect(snapshotChanges(known, hidden, now, "h24")).toEqual([]);
    const without: Snapshot = { ...known, providers: [] };
    expect(snapshotChanges(without, warn, now, "h24")).toEqual([]);
  });

  it("ignores elapsed windows and unknown levels", () => {
    expect(
      snapshotChanges(fixtures.elapsed.snapshot, known, now, "h24"),
    ).toEqual([]);
  });
});
