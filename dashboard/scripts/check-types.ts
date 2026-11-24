// The independent type-discipline check (docs/dashboard/DESIGN.md 1a and 10, point 3).
//
// It uses the TypeScript compiler API and `svelte/compiler` directly (no ESLint involved)
// and fails on every construct the lint layer bans: the keyword types, assertions, non-null
// and definite `!`, type predicates, `declare`, `JSON.parse`, `@ts-` directives, a
// `<script>` that is not TypeScript, a `svelte-ignore` other than the approved one, any
// tracked JavaScript file, and any value whose INFERRED type is the untyped one.
//
// Run: `node scripts/check-types.ts` (Node strips the types itself; no loader).

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "svelte/compiler";
import ts from "typescript";

/** What a finding is about. */
export type Rule =
  | "keyword-any"
  | "keyword-unknown"
  | "keyword-never"
  | "banned-type"
  | "assertion"
  | "non-null"
  | "definite"
  | "ts-directive"
  | "catch-binding"
  | "rejection-parameter"
  | "type-predicate"
  | "declare"
  | "json-parse"
  | "dom-type-argument"
  | "inferred-untyped"
  | "script-lang"
  | "svelte-ignore"
  | "svelte-import"
  | "js-file"
  | "unreadable";

/** One violation. */
export type Finding = { file: string; line: number; rule: Rule };

/** The single `svelte-ignore` the design approves (DESIGN 9.3), at most once in the tree. */
const APPROVED_IGNORE = "a11y_no_noninteractive_tabindex";

/** The one ambient file that may hold `declare` (it only references Vite's and Svelte's types). */
const AMBIENT_FILE = "src/env.d.ts";

const DOM_LOOKUPS: readonly string[] = [
  "querySelector",
  "querySelectorAll",
  "closest",
  "getElementById",
];

const DIRECTIVE = /@ts-(?:ignore|expect-error|nocheck|check)\b/;

// ---------------------------------------------------------------------------------------
// TypeScript files: the syntax pass
// ---------------------------------------------------------------------------------------

function lineOf(source: ts.SourceFile, position: number): number {
  return source.getLineAndCharacterOfPosition(position).line + 1;
}

function hasDeclareModifier(node: ts.Node): boolean {
  if (!ts.canHaveModifiers(node)) {
    return false;
  }
  const modifiers = ts.getModifiers(node);
  if (modifiers === undefined) {
    return false;
  }
  return modifiers.some((m) => m.kind === ts.SyntaxKind.DeclareKeyword);
}

function isFunctionWithParameters(node: ts.Node | undefined): boolean {
  if (node === undefined) {
    return false;
  }
  if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
    return node.parameters.length > 0;
  }
  return false;
}

function callRules(node: ts.CallExpression): Rule[] {
  const rules: Rule[] = [];
  const callee = node.expression;
  if (!ts.isPropertyAccessExpression(callee)) {
    return rules;
  }
  const name = callee.name.text;
  if (
    name === "parse" &&
    ts.isIdentifier(callee.expression) &&
    callee.expression.text === "JSON"
  ) {
    rules.push("json-parse");
  }
  if (name === "json" && node.arguments.length === 0) {
    rules.push("json-parse");
  }
  if (DOM_LOOKUPS.includes(name) && node.typeArguments !== undefined) {
    rules.push("dom-type-argument");
  }
  if (name === "catch" && isFunctionWithParameters(node.arguments[0])) {
    rules.push("rejection-parameter");
  }
  if (name === "then" && isFunctionWithParameters(node.arguments[1])) {
    rules.push("rejection-parameter");
  }
  return rules;
}

function nodeRules(node: ts.Node, ambient: boolean): Rule[] {
  const rules: Rule[] = [];
  switch (node.kind) {
    case ts.SyntaxKind.AnyKeyword:
      rules.push("keyword-any");
      break;
    case ts.SyntaxKind.UnknownKeyword:
      rules.push("keyword-unknown");
      break;
    case ts.SyntaxKind.NeverKeyword:
      rules.push("keyword-never");
      break;
    case ts.SyntaxKind.ObjectKeyword:
      rules.push("banned-type");
      break;
    case ts.SyntaxKind.TypeAssertionExpression:
      rules.push("assertion");
      break;
    case ts.SyntaxKind.NonNullExpression:
      rules.push("non-null");
      break;
    case ts.SyntaxKind.TypePredicate:
      rules.push("type-predicate");
      break;
    default:
      break;
  }
  if (ts.isAsExpression(node) && !ts.isConstTypeReference(node.type)) {
    rules.push("assertion");
  }
  if (
    (ts.isVariableDeclaration(node) || ts.isPropertyDeclaration(node)) &&
    node.exclamationToken !== undefined
  ) {
    rules.push("definite");
  }
  if (ts.isCatchClause(node) && node.variableDeclaration !== undefined) {
    rules.push("catch-binding");
  }
  if (!ambient && hasDeclareModifier(node)) {
    rules.push("declare");
  }
  if (ts.isCallExpression(node)) {
    rules.push(...callRules(node));
  }
  if (ts.isTypeReferenceNode(node) && ts.isIdentifier(node.typeName)) {
    const name = node.typeName.text;
    if (name === "Function" || name === "Object") {
      rules.push("banned-type");
    }
  }
  if (ts.isTypeLiteralNode(node) && node.members.length === 0) {
    rules.push("banned-type");
  }
  return rules;
}

function directiveLines(source: ts.SourceFile): number[] {
  const text = source.getFullText();
  const seen = new Set<number>();
  const lines: number[] = [];
  const scan = (ranges: readonly ts.CommentRange[] | undefined): void => {
    if (ranges === undefined) {
      return;
    }
    for (const range of ranges) {
      if (seen.has(range.pos)) {
        continue;
      }
      seen.add(range.pos);
      if (DIRECTIVE.test(text.slice(range.pos, range.end))) {
        lines.push(lineOf(source, range.pos));
      }
    }
  };
  const visit = (node: ts.Node): void => {
    scan(ts.getLeadingCommentRanges(text, node.getFullStart()));
    scan(ts.getTrailingCommentRanges(text, node.getEnd()));
    for (const child of node.getChildren(source)) {
      visit(child);
    }
  };
  visit(source);
  return lines;
}

function checkTypeScriptSyntax(file: string, text: string): Finding[] {
  const findings: Finding[] = [];
  const source = ts.createSourceFile(
    file,
    text,
    ts.ScriptTarget.ES2022,
    true,
    ts.ScriptKind.TS,
  );
  const ambient = file === AMBIENT_FILE;
  const visit = (node: ts.Node): void => {
    for (const rule of nodeRules(node, ambient)) {
      findings.push({
        file,
        line: lineOf(source, node.getStart(source)),
        rule,
      });
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  for (const line of directiveLines(source)) {
    findings.push({ file, line, rule: "ts-directive" });
  }
  return findings;
}

// ---------------------------------------------------------------------------------------
// TypeScript files: the inferred-type pass
// ---------------------------------------------------------------------------------------

const UNTYPED = ts.TypeFlags.Any | ts.TypeFlags.Unknown;

function isUntyped(type: ts.Type): boolean {
  return (type.flags & UNTYPED) !== 0;
}

function isCheckedDeclaration(node: ts.Node): boolean {
  return (
    ts.isVariableDeclaration(node) ||
    ts.isParameter(node) ||
    ts.isPropertyDeclaration(node) ||
    ts.isPropertySignature(node) ||
    ts.isBindingElement(node)
  );
}

function asFunctionLike(node: ts.Node): ts.SignatureDeclaration | null {
  if (
    ts.isFunctionDeclaration(node) ||
    ts.isFunctionExpression(node) ||
    ts.isArrowFunction(node) ||
    ts.isMethodDeclaration(node) ||
    ts.isMethodSignature(node)
  ) {
    return node;
  }
  return null;
}

function checkInferredTypes(
  root: string,
  files: readonly string[],
  options: ts.CompilerOptions,
): Finding[] {
  const findings: Finding[] = [];
  const absolute = new Map<string, string>();
  for (const file of files) {
    absolute.set(resolve(root, file), file);
  }
  const program = ts.createProgram([...absolute.keys()], options);
  const checker = program.getTypeChecker();
  for (const source of program.getSourceFiles()) {
    const file = absolute.get(resolve(source.fileName));
    if (file === undefined || source.isDeclarationFile) {
      continue;
    }
    const reported = new Set<number>();
    const report = (node: ts.Node): void => {
      const line = lineOf(source, node.getStart(source));
      if (!reported.has(line)) {
        reported.add(line);
        findings.push({ file, line, rule: "inferred-untyped" });
      }
    };
    const visit = (node: ts.Node): void => {
      // Type positions hold no value; an import or re-export only names things, and each
      // imported value is checked where it is used.
      if (
        ts.isPartOfTypeNode(node) ||
        ts.isImportDeclaration(node) ||
        ts.isExportDeclaration(node)
      ) {
        return;
      }
      if (ts.isExpression(node) || isCheckedDeclaration(node)) {
        if (isUntyped(checker.getTypeAtLocation(node))) {
          report(node);
        }
      }
      const fn = asFunctionLike(node);
      if (fn !== null) {
        const signature = checker.getSignatureFromDeclaration(fn);
        if (signature !== undefined && isUntyped(signature.getReturnType())) {
          report(node);
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  return findings;
}

// ---------------------------------------------------------------------------------------
// Svelte files: the whole file, markup included
// ---------------------------------------------------------------------------------------

/** A parsed Svelte tree as plain data: what `JSON.stringify` walks. */
type Tree =
  | string
  | number
  | boolean
  | bigint
  | null
  | undefined
  | Tree[]
  | { readonly [key: string]: Tree };

type TreeNode = Readonly<Record<string, Tree>>;

function asNode(value: Tree): TreeNode | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  return value;
}

function field(node: TreeNode | null, key: string): Tree {
  return node === null ? undefined : node[key];
}

function isOwnImport(specifier: Tree): boolean {
  if (typeof specifier !== "string") {
    return false;
  }
  return (
    specifier === "svelte" ||
    specifier.startsWith("svelte/") ||
    specifier.startsWith("./") ||
    specifier.startsWith("../")
  );
}

function hasParameters(value: Tree): boolean {
  const node = asNode(value);
  const type = field(node, "type");
  if (type !== "ArrowFunctionExpression" && type !== "FunctionExpression") {
    return false;
  }
  const params = field(node, "params");
  return Array.isArray(params) && params.length > 0;
}

function svelteCallRules(node: TreeNode): Rule[] {
  const rules: Rule[] = [];
  const callee = asNode(field(node, "callee"));
  if (field(callee, "type") !== "MemberExpression") {
    return rules;
  }
  const name = field(asNode(field(callee, "property")), "name");
  const object = asNode(field(callee, "object"));
  const args = field(node, "arguments");
  const list: Tree[] = Array.isArray(args) ? args : [];
  if (name === "parse" && field(object, "name") === "JSON") {
    rules.push("json-parse");
  }
  if (name === "json" && list.length === 0) {
    rules.push("json-parse");
  }
  const typed =
    asNode(field(node, "typeArguments")) !== null ||
    asNode(field(node, "typeParameters")) !== null;
  if (typeof name === "string" && DOM_LOOKUPS.includes(name) && typed) {
    rules.push("dom-type-argument");
  }
  if (name === "catch" && hasParameters(list[0])) {
    rules.push("rejection-parameter");
  }
  if (name === "then" && hasParameters(list[1])) {
    rules.push("rejection-parameter");
  }
  return rules;
}

function scriptIsTypeScript(node: TreeNode): boolean {
  const attributes = field(node, "attributes");
  if (!Array.isArray(attributes)) {
    return false;
  }
  return attributes.some((attribute) => {
    const entry = asNode(attribute);
    if (field(entry, "name") !== "lang") {
      return false;
    }
    const value = field(entry, "value");
    if (!Array.isArray(value)) {
      return false;
    }
    return value.some((part) => field(asNode(part), "data") === "ts");
  });
}

function svelteNodeRules(node: TreeNode): Rule[] {
  const rules: Rule[] = [];
  const type = field(node, "type");
  switch (type) {
    case "TSAnyKeyword":
      rules.push("keyword-any");
      break;
    case "TSUnknownKeyword":
      rules.push("keyword-unknown");
      break;
    case "TSNeverKeyword":
      rules.push("keyword-never");
      break;
    case "TSObjectKeyword":
      rules.push("banned-type");
      break;
    case "TSTypeAssertion":
      rules.push("assertion");
      break;
    case "TSNonNullExpression":
      rules.push("non-null");
      break;
    case "TSTypePredicate":
      rules.push("type-predicate");
      break;
    case "TSAsExpression": {
      const annotation = asNode(field(node, "typeAnnotation"));
      const name = field(asNode(field(annotation, "typeName")), "name");
      if (name !== "const") {
        rules.push("assertion");
      }
      break;
    }
    case "TSTypeReference": {
      const name = field(asNode(field(node, "typeName")), "name");
      if (name === "Function" || name === "Object") {
        rules.push("banned-type");
      }
      break;
    }
    case "TSTypeLiteral": {
      const members = field(node, "members");
      if (Array.isArray(members) && members.length === 0) {
        rules.push("banned-type");
      }
      break;
    }
    case "CatchClause":
      if (asNode(field(node, "param")) !== null) {
        rules.push("catch-binding");
      }
      break;
    case "CallExpression":
      rules.push(...svelteCallRules(node));
      break;
    case "Script":
      if (!scriptIsTypeScript(node)) {
        rules.push("script-lang");
      }
      break;
    case "ImportDeclaration":
    case "ImportExpression":
      if (!isOwnImport(field(asNode(field(node, "source")), "value"))) {
        rules.push("svelte-import");
      }
      break;
    default:
      break;
  }
  if (field(node, "definite") === true) {
    rules.push("definite");
  }
  if (field(node, "declare") === true) {
    rules.push("declare");
  }
  return rules;
}

function lineAt(text: string, offset: number): number {
  let line = 1;
  for (let index = 0; index < offset && index < text.length; index += 1) {
    if (text[index] === "\n") {
      line += 1;
    }
  }
  return line;
}

/** The `svelte-ignore` codes of one file, with their lines. */
function ignoreComments(text: string): { line: number; code: string }[] {
  const found: { line: number; code: string }[] = [];
  const pattern = /svelte-ignore\s+([A-Za-z0-9_-]*)/g;
  for (const match of text.matchAll(pattern)) {
    found.push({ line: lineAt(text, match.index), code: match[1] ?? "" });
  }
  return found;
}

function checkSvelte(file: string, text: string): Finding[] {
  const findings: Finding[] = [];
  const root = parse(text, { modern: true });
  let offset = 0;
  const visit = (_key: string, value: Tree): Tree => {
    if (typeof value === "bigint") {
      return String(value);
    }
    const node = asNode(value);
    if (node === null) {
      return value;
    }
    const start = field(node, "start");
    if (typeof start === "number") {
      offset = start;
    }
    for (const rule of svelteNodeRules(node)) {
      findings.push({ file, line: lineAt(text, offset), rule });
    }
    return value;
  };
  JSON.stringify(root, visit);
  const lines = text.split("\n");
  lines.forEach((content, index) => {
    const comment =
      /(?:\/\/|\/\*|<!--).*@ts-(?:ignore|expect-error|nocheck|check)\b/;
    if (comment.test(content)) {
      findings.push({ file, line: index + 1, rule: "ts-directive" });
    }
  });
  return findings;
}

// ---------------------------------------------------------------------------------------
// The project
// ---------------------------------------------------------------------------------------

function isJavaScript(file: string): boolean {
  return file.endsWith(".js") || file.endsWith(".mjs") || file.endsWith(".cjs");
}

function compare(a: Finding, b: Finding): number {
  if (a.file !== b.file) {
    return a.file < b.file ? -1 : 1;
  }
  if (a.line !== b.line) {
    return a.line - b.line;
  }
  return a.rule < b.rule ? -1 : a.rule > b.rule ? 1 : 0;
}

/**
 * Checks `files` (paths relative to `root`): every `.ts` and `.svelte` file for the banned
 * syntax, the `.ts` files for inferred untyped values, and the list itself for JavaScript.
 */
export function checkProject(
  root: string,
  files: readonly string[],
  options: ts.CompilerOptions,
): Finding[] {
  const findings: Finding[] = [];
  const typed: string[] = [];
  let approvedIgnores = 0;
  for (const file of files) {
    if (isJavaScript(file)) {
      findings.push({ file, line: 1, rule: "js-file" });
      continue;
    }
    const isSvelte = file.endsWith(".svelte");
    if (!isSvelte && !file.endsWith(".ts")) {
      continue;
    }
    const text = readFileSync(join(root, file), "utf8");
    if (!isSvelte) {
      typed.push(file);
      findings.push(...checkTypeScriptSyntax(file, text));
      continue;
    }
    try {
      findings.push(...checkSvelte(file, text));
    } catch {
      findings.push({ file, line: 1, rule: "unreadable" });
    }
    for (const ignore of ignoreComments(text)) {
      if (ignore.code === APPROVED_IGNORE && approvedIgnores === 0) {
        approvedIgnores = 1;
      } else {
        findings.push({ file, line: ignore.line, rule: "svelte-ignore" });
      }
    }
  }
  findings.push(...checkInferredTypes(root, typed, options));
  return findings.sort(compare);
}

/** Directories that hold build output or installed packages, never source. */
const NOT_SOURCE: readonly string[] = [
  "node_modules/",
  "dist/",
  "coverage/",
  "test-results/",
  "playwright-report/",
  "src-tauri/target/",
  "src-tauri/gen/",
];

/** Every tracked or not-yet-ignored file under `root` (build output is not source). */
export function sourceFiles(root: string): string[] {
  const listed = execFileSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard"],
    { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  return listed
    .split("\n")
    .filter((file) => file !== "")
    .filter((file) => !NOT_SOURCE.some((prefix) => file.startsWith(prefix)))
    .sort();
}

/** The compiler options of the project's `tsconfig.json`. */
export function projectOptions(root: string): ts.CompilerOptions {
  const parsed = ts.getParsedCommandLineOfConfigFile(
    join(root, "tsconfig.json"),
    {},
    {
      ...ts.sys,
      onUnRecoverableConfigFileDiagnostic: () => {
        return;
      },
    },
  );
  return parsed === undefined ? {} : parsed.options;
}

function main(): void {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const files = sourceFiles(root);
  const findings = checkProject(root, files, projectOptions(root));
  for (const finding of findings) {
    process.stdout.write(
      `${finding.file}:${String(finding.line)}  ${finding.rule}\n`,
    );
  }
  const checked = files.filter(
    (file) => file.endsWith(".ts") || file.endsWith(".svelte"),
  ).length;
  process.stdout.write(
    `check-types: ${String(checked)} files, ${String(findings.length)} findings\n`,
  );
  process.exitCode = findings.length === 0 ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
