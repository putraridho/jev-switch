// One source file per banned construct (docs/dashboard/TASKS.md T00, "Type discipline"),
// kept as strings so that this file itself stays clean. The self-tests write them into a
// temporary directory and expect each one to be reported by `check-types.ts` and by ESLint.

import type { Rule } from "./check-types.ts";

/** One file that must be reported. */
export type BannedSample = {
  /** What the sample shows. */
  name: string;
  /** The file name it is written under. */
  file: string;
  /** Its source text. */
  code: string;
  /** The finding `check-types.ts` must report for it. */
  rule: Rule;
  /** The ESLint rule that must report it; `null` where only `check-types.ts` can. */
  eslint: string | null;
};

const SYNTAX = "no-restricted-syntax";

export const bannedSamples: readonly BannedSample[] = [
  {
    name: "the any keyword",
    file: "any.ts",
    code: "export const a: any = 1;\n",
    rule: "keyword-any",
    eslint: "@typescript-eslint/no-explicit-any",
  },
  {
    name: "the unknown keyword",
    file: "unknown.ts",
    code: "export function f(x: unknown): number {\n  return x === 1 ? 1 : 0;\n}\n",
    rule: "keyword-unknown",
    eslint: SYNTAX,
  },
  {
    name: "the never keyword",
    file: "never.ts",
    code: "export function f(x: never): number {\n  return x;\n}\n",
    rule: "keyword-never",
    eslint: SYNTAX,
  },
  {
    name: "an `as` assertion",
    file: "as.ts",
    code: "const x: string | number = Math.random() > 2 ? 1 : 'a';\nexport const y = x as string;\n",
    rule: "assertion",
    eslint: "@typescript-eslint/consistent-type-assertions",
  },
  {
    name: "an angle-bracket assertion",
    file: "angle.ts",
    code: "const x: string | number = Math.random() > 2 ? 1 : 'a';\nexport const y = <string>x;\n",
    rule: "assertion",
    eslint: "@typescript-eslint/consistent-type-assertions",
  },
  {
    name: "a non-null assertion",
    file: "nonnull.ts",
    code: "const x: string | null = Math.random() > 2 ? null : 'a';\nexport const y = x!;\n",
    rule: "non-null",
    eslint: "@typescript-eslint/no-non-null-assertion",
  },
  {
    name: "a definite assignment assertion",
    file: "definite.ts",
    code: "let x!: number;\nexport function f(): number {\n  return x;\n}\n",
    rule: "definite",
    eslint: SYNTAX,
  },
  {
    name: "a @ts-ignore directive",
    file: "directive.ts",
    code: "// @ts-ignore\nexport const x: number = 1;\n",
    rule: "ts-directive",
    eslint: "@typescript-eslint/ban-ts-comment",
  },
  {
    name: "a catch binding",
    file: "catch.ts",
    code: "export function f(): number {\n  try {\n    return 1;\n  } catch (e) {\n    return e === 1 ? 1 : 0;\n  }\n}\n",
    rule: "catch-binding",
    eslint: SYNTAX,
  },
  {
    name: "a rejection callback with a parameter",
    file: "rejection.ts",
    code: "export const p = Promise.resolve(1).catch((e) => (e === 1 ? 1 : 0));\n",
    rule: "rejection-parameter",
    eslint: SYNTAX,
  },
  {
    name: "a type predicate",
    file: "predicate.ts",
    code: "export function isText(x: string | number): x is string {\n  return typeof x === 'string';\n}\n",
    rule: "type-predicate",
    eslint: SYNTAX,
  },
  {
    name: "a declare const",
    file: "declare.ts",
    code: "declare const injected: number;\nexport const x = injected;\n",
    rule: "declare",
    eslint: SYNTAX,
  },
  {
    name: "JSON.parse",
    file: "jsonparse.ts",
    code: "export const text = String(JSON.parse('1'));\n",
    rule: "json-parse",
    eslint: SYNTAX,
  },
  {
    name: "a type argument on querySelector",
    file: "queryselector.ts",
    code: "export const el = document.querySelector<HTMLInputElement>('input');\n",
    rule: "dom-type-argument",
    eslint: SYNTAX,
  },
  {
    name: "a value inferred as any from a third-party type",
    file: "inferred.ts",
    code: "export const state: number = history.state;\n",
    rule: "inferred-untyped",
    eslint: "@typescript-eslint/no-unsafe-assignment",
  },
  {
    name: "an `as` inside the markup of a component",
    file: "Markup.svelte",
    code: '<script lang="ts">\n  const value: string | number = "a";\n</script>\n\n<p>{value as string}</p>\n',
    rule: "assertion",
    eslint: "@typescript-eslint/consistent-type-assertions",
  },
  {
    name: "a second svelte-ignore",
    file: "Ignore.svelte",
    code: '<script lang="ts">\n  const label = "a";\n</script>\n\n<!-- svelte-ignore a11y_no_noninteractive_tabindex -->\n<p>{label}</p>\n<!-- svelte-ignore a11y_no_noninteractive_tabindex -->\n<p>{label}</p>\n',
    rule: "svelte-ignore",
    eslint: null,
  },
  {
    name: "a script block that is not TypeScript",
    file: "Lang.svelte",
    code: '<script>\n  const label = "a";\n</script>\n\n<p>{label}</p>\n',
    rule: "script-lang",
    eslint: "svelte/block-lang",
  },
  {
    name: "a component that imports a third-party package",
    file: "Import.svelte",
    code: '<script lang="ts">\n  import { invoke } from "@tauri-apps/api/core";\n  const call = invoke;\n</script>\n\n<p>{typeof call}</p>\n',
    rule: "svelte-import",
    eslint: "no-restricted-imports",
  },
  {
    name: "a tracked JavaScript file",
    file: "legacy.js",
    code: "export const x = 1;\n",
    rule: "js-file",
    eslint: null,
  },
];

/** Constructs that look similar and are allowed: none of these may be reported. */
export const allowedSamples: readonly {
  name: string;
  file: string;
  code: string;
}[] = [
  {
    name: "as const",
    file: "ok-const.ts",
    code: "export const levels = ['ok', 'warn'] as const;\n",
  },
  {
    name: "satisfies",
    file: "ok-satisfies.ts",
    code: "type Level = 'ok' | 'warn';\nexport const words = { ok: 'Fine', warn: 'High' } satisfies Record<Level, string>;\n",
  },
  {
    name: "the banned words inside strings and comments",
    file: "ok-words.ts",
    code: "// This sentence says never, any and unknown, and is not a finding.\nexport const copy = 'never shows a guess; any unknown window is named as is; @ts-ignore';\n",
  },
  {
    name: "a rejection callback without a parameter",
    file: "ok-rejection.ts",
    code: "export const p = Promise.resolve(1).catch(() => 0);\n",
  },
  {
    name: "the one approved svelte-ignore, once",
    file: "OkIgnore.svelte",
    code: '<script lang="ts">\n  const label = "a";\n</script>\n\n<!-- svelte-ignore a11y_no_noninteractive_tabindex -->\n<p tabindex="0">{label}</p>\n',
  },
  {
    name: "a component importing svelte and its own modules",
    file: "OkImport.svelte",
    code: '<script lang="ts">\n  import { onMount } from "svelte";\n  import { levels } from "./ok-const";\n  onMount(() => {\n    return;\n  });\n</script>\n\n<p>{levels.length}</p>\n',
  },
];
