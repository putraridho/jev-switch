// The lint layer reports the same constructs as `check-types.ts` (docs/dashboard/TASKS.md
// T00). The samples are written into a scratch directory inside `src/`, so they are part
// of the type-checked project, linted with the real configuration, and removed again.

import { ESLint } from "eslint";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { allowedSamples, bannedSamples } from "./banned-samples.ts";

const project = join(import.meta.dirname, "..");
const scratch = join(project, "src", "__lint_selftest__");

/** The rule ids reported per file name, and whether the file could not be linted at all. */
const reported = new Map<string, { rules: string[]; fatal: boolean }>();

beforeAll(async () => {
  rmSync(scratch, { recursive: true, force: true });
  mkdirSync(scratch, { recursive: true });
  for (const sample of [...bannedSamples, ...allowedSamples]) {
    writeFileSync(join(scratch, sample.file), sample.code);
  }
  const eslint = new ESLint({ cwd: project, allowInlineConfig: false });
  const results = await eslint.lintFiles([join(scratch, "**")]);
  for (const result of results) {
    const rules: string[] = [];
    let fatal = false;
    for (const message of result.messages) {
      if (message.ruleId !== null) {
        rules.push(message.ruleId);
      }
      if (message.fatal === true) {
        fatal = true;
      }
    }
    reported.set(result.filePath, { rules, fatal });
  }
});

afterAll(() => {
  rmSync(scratch, { recursive: true, force: true });
});

describe("eslint", () => {
  const linted = bannedSamples.filter((sample) => sample.eslint !== null);

  it.each(linted.map((sample) => [sample.name, sample] as const))(
    "reports %s",
    (_name, sample) => {
      const result = reported.get(join(scratch, sample.file));
      expect(result?.rules).toContain(sample.eslint);
    },
  );

  it("cannot lint a JavaScript file as part of the project", () => {
    const result = reported.get(join(scratch, "legacy.js"));
    expect(result?.fatal).toBe(true);
  });

  it.each(allowedSamples.map((sample) => [sample.name, sample] as const))(
    "accepts %s",
    (_name, sample) => {
      const result = reported.get(join(scratch, sample.file));
      expect(result).toEqual({ rules: [], fatal: false });
    },
  );
});
