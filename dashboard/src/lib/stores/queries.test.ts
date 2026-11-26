import { describe, expect, it } from "vitest";
import { fixtures } from "../ipc/fixtures";
import { createMockClient } from "../ipc/mock";
import { initQueries, queries } from "./queries";

const query = {
  provider: "claude",
  meter_id: "w:5h",
  range: "day",
} as const;

// The module keeps its client for the life of the file, so "before wired" comes first.
describe("queries", () => {
  it("answers with an error before it is wired", async () => {
    expect(await queries.history(query)).toEqual({
      status: "error",
      error: { code: "internal" },
    });
    expect((await queries.diagnostics()).status).toBe("error");
  });

  it("asks the client once it is wired", async () => {
    const client = createMockClient(structuredClone(fixtures.known));
    initQueries(client);
    const history = await queries.history(query);
    expect(history.status).toBe("ok");
    expect((await queries.diagnostics()).status).toBe("ok");
    expect(client.calls.map((c) => c.command)).toEqual([
      "getHistory",
      "getDiagnostics",
    ]);
  });
});
