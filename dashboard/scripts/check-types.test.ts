// The self-test of `check-types.ts` (docs/dashboard/TASKS.md T00): every banned construct
// is reported, the look-alikes are not.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import ts from "typescript";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { allowedSamples, bannedSamples } from "./banned-samples.ts";
import {
  checkProject,
  projectOptions,
  sourceFiles,
  type Finding,
} from "./check-types.ts";

const options: ts.CompilerOptions = {
  target: ts.ScriptTarget.ES2022,
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  lib: ["lib.es2022.d.ts", "lib.dom.d.ts"],
  strict: true,
  noEmit: true,
  types: [],
};

let root = "";
let findings: Finding[] = [];

beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), "flashboard-check-types-"));
  mkdirSync(join(root, "banned"));
  mkdirSync(join(root, "allowed"));
  const banned: string[] = [];
  for (const sample of bannedSamples) {
    const file = join("banned", sample.file);
    writeFileSync(join(root, file), sample.code);
    banned.push(file);
  }
  const allowed: string[] = [];
  for (const sample of allowedSamples) {
    const file = join("allowed", sample.file);
    writeFileSync(join(root, file), sample.code);
    allowed.push(file);
  }
  // Two runs: the one approved svelte-ignore is counted once per project.
  findings = [
    ...checkProject(root, banned, options),
    ...checkProject(root, allowed, options),
  ];
});

afterAll(() => {
  rmSync(root, { recursive: true, force: true });
});

describe("check-types", () => {
  it.each(bannedSamples.map((sample) => [sample.name, sample] as const))(
    "reports %s",
    (_name, sample) => {
      const rules = findings
        .filter((finding) => finding.file === join("banned", sample.file))
        .map((finding) => finding.rule);
      expect(rules).toContain(sample.rule);
    },
  );

  it.each(allowedSamples.map((sample) => [sample.name, sample] as const))(
    "accepts %s",
    (_name, sample) => {
      const hits = findings.filter(
        (finding) => finding.file === join("allowed", sample.file),
      );
      expect(hits).toEqual([]);
    },
  );

  it("covers every construct the task lists", () => {
    expect(bannedSamples.length).toBeGreaterThanOrEqual(19);
    const rules = new Set(bannedSamples.map((sample) => sample.rule));
    expect([...rules].sort()).toEqual(
      [
        "assertion",
        "catch-binding",
        "declare",
        "definite",
        "dom-type-argument",
        "inferred-untyped",
        "js-file",
        "json-parse",
        "keyword-any",
        "keyword-never",
        "keyword-unknown",
        "non-null",
        "rejection-parameter",
        "script-lang",
        "svelte-ignore",
        "svelte-import",
        "ts-directive",
        "type-predicate",
      ].sort(),
    );
  });

  it("finds nothing in the project itself", () => {
    const project = join(import.meta.dirname, "..");
    const files = sourceFiles(project);
    expect(files).toContain("src/main.ts");
    expect(files).toContain("src/lib/ipc/bindings/Snapshot.ts");
    expect(files).toContain("eslint.config.ts");
    expect(files.some((file) => file.startsWith("node_modules/"))).toBe(false);
    expect(checkProject(project, files, projectOptions(project))).toEqual([]);
  });
});
