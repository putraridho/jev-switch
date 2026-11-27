import { describe, expect, it } from "vitest";
import { modelLabel, parseRouteNote, routeSentence } from "./route";

describe("route notes", () => {
  it("reads the engine's routing note", () => {
    expect(
      parseRouteNote("task: feature (0.88) -> claude claude-sonnet-5-5"),
    ).toEqual({
      task: "feature",
      confidence: 0.88,
      provider: "claude",
      model: "claude-sonnet-5-5",
    });
  });

  it("writes it as a sentence", () => {
    expect(
      routeSentence("task: feature (0.88) -> claude claude-sonnet-5-5"),
    ).toBe("A feature task, so Claude · 88% sure");
    expect(
      routeSentence("task: explain (0.6) -> gemini gemini-4-hydrogen"),
    ).toBe("An explain task, so Gemini · 60% sure");
    expect(routeSentence("task: bug_fix (1.4) -> codex gpt-6.1-sol")).toBe(
      "A bug fix task, so Codex · 100% sure",
    );
  });

  it("leaves any other note as written", () => {
    expect(routeSentence("pinned to Claude")).toBe("pinned to Claude");
    expect(routeSentence("task: feature (0.9) -> nobody m")).toBe(
      "task: feature (0.9) -> nobody m",
    );
  });

  it("names models the way people say them", () => {
    expect(modelLabel("claude", "claude-sonnet-5-5")).toBe("Sonnet 5.5");
    expect(modelLabel("codex", "gpt-6.1-sol")).toBe("GPT 6.1 Sol");
    expect(modelLabel("gemini", "gemini-4-hydrogen")).toBe("4 Hydrogen");
    expect(modelLabel("deepseek", "deepseek-v4-pro")).toBe("V4 Pro");
    expect(modelLabel(null, "m")).toBe("M");
  });
});
