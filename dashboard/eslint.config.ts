// Flashboard's lint layer (docs/dashboard/DESIGN.md 1a, 8.10, 10). Run with
// `--no-inline-config --max-warnings 0`, so no rule can be switched off in a file.

import { defineConfig } from "eslint/config";
import svelte from "eslint-plugin-svelte";
import tseslint from "typescript-eslint";

const REASON_TYPES =
  "Every type is defined: no any, unknown or never (DESIGN 1a).";
const REASON_ASSERT =
  "No assertion, in any form: narrow instead (DESIGN 1a, 10).";
const REASON_HTML = "No raw-HTML sink (DESIGN 8.10).";
const REASON_STORAGE = "The web view stores nothing (DESIGN 8.4).";
const REASON_STYLE =
  "The CSP blocks style attributes: use the style: directive (DESIGN 8.8).";

/** The AST nodes the type discipline bans, in `.ts`, `.svelte.ts` and `.svelte`. */
const bannedSyntax = [
  { selector: "TSAnyKeyword", message: REASON_TYPES },
  { selector: "TSUnknownKeyword", message: REASON_TYPES },
  { selector: "TSNeverKeyword", message: REASON_TYPES },
  { selector: "TSTypeAssertion", message: REASON_ASSERT },
  {
    selector:
      "TSAsExpression:not([typeAnnotation.type='TSTypeReference'][typeAnnotation.typeName.name='const'])",
    message: REASON_ASSERT,
  },
  { selector: "TSNonNullExpression", message: REASON_ASSERT },
  {
    selector: "CatchClause[param]",
    message: "No catch binding: failures arrive as typed Outcome values.",
  },
  { selector: "TSTypePredicate", message: REASON_ASSERT },
  { selector: "VariableDeclaration[declare=true]", message: REASON_ASSERT },
  { selector: "TSDeclareFunction[declare=true]", message: REASON_ASSERT },
  { selector: "ClassDeclaration[declare=true]", message: REASON_ASSERT },
  { selector: "TSModuleDeclaration[declare=true]", message: REASON_ASSERT },
  { selector: "TSEnumDeclaration", message: "erasableSyntaxOnly: no enum." },
  { selector: "VariableDeclarator[definite=true]", message: REASON_ASSERT },
  { selector: "PropertyDefinition[definite=true]", message: REASON_ASSERT },
  {
    selector:
      "CallExpression[typeArguments][callee.property.name=/^(querySelector|querySelectorAll|closest|getElementById)$/]",
    message: "Narrow a DOM lookup with instanceof, not a type argument.",
  },
  {
    selector:
      "CallExpression[callee.object.name='JSON'][callee.property.name='parse']",
    message: "No JSON.parse: its result is untyped (DESIGN 10).",
  },
  {
    selector: "CallExpression[callee.property.name='json'][arguments.length=0]",
    message: "No .json(): its result is untyped (DESIGN 10).",
  },
  {
    selector:
      "CallExpression[callee.property.name='catch'] > :matches(ArrowFunctionExpression, FunctionExpression).arguments[params.length>0]",
    message: "A rejection callback takes no parameter: the value is untyped.",
  },
  {
    selector:
      "CallExpression[callee.property.name='then'] > :matches(ArrowFunctionExpression, FunctionExpression).arguments:nth-child(2)[params.length>0]",
    message: "A rejection callback takes no parameter: the value is untyped.",
  },
  {
    selector: "MemberExpression[property.name=/^(innerHTML|outerHTML)$/]",
    message: REASON_HTML,
  },
  {
    selector: "CallExpression[callee.property.name='insertAdjacentHTML']",
    message: REASON_HTML,
  },
  {
    selector: "MemberExpression[property.name='cookie']",
    message: REASON_STORAGE,
  },
  {
    selector:
      "MemberExpression[property.name=/^(localStorage|sessionStorage|indexedDB|caches)$/]",
    message: REASON_STORAGE,
  },
  {
    selector: "MemberExpression[property.name='cssText']",
    message: REASON_STYLE,
  },
  {
    selector:
      "CallExpression[callee.property.name='setAttribute'][arguments.0.value='style']",
    message: REASON_STYLE,
  },
  { selector: "SvelteAttribute[key.name='style']", message: REASON_STYLE },
];

export default defineConfig(
  {
    ignores: [
      "dist/",
      "dist-gallery/",
      "node_modules/",
      "coverage/",
      "test-results/",
      "playwright-report/",
      "src-tauri/",
      "crates/",
      "fixtures/",
      "e2e/smoke/",
    ],
  },
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: [".svelte"],
      },
    },
    linterOptions: {
      reportUnusedDisableDirectives: "error",
    },
  },
  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: [".svelte"],
        // Svelte's options live in vite.config.ts; there is no svelte.config file.
        svelteConfig: { compilerOptions: { runes: true } },
      },
    },
  },
  {
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unsafe-argument": "error",
      "@typescript-eslint/no-unsafe-assignment": "error",
      "@typescript-eslint/no-unsafe-call": "error",
      "@typescript-eslint/no-unsafe-member-access": "error",
      "@typescript-eslint/no-unsafe-return": "error",
      "@typescript-eslint/consistent-type-assertions": [
        "error",
        { assertionStyle: "never" },
      ],
      "@typescript-eslint/no-non-null-assertion": "error",
      "@typescript-eslint/ban-ts-comment": [
        "error",
        {
          "ts-check": true,
          "ts-expect-error": true,
          "ts-ignore": true,
          "ts-nocheck": true,
        },
      ],
      "@typescript-eslint/no-restricted-types": [
        "error",
        {
          types: {
            Function: "Write the function type out.",
            Object: "Write the object type out.",
            object: "Write the object type out.",
            "{}": "Write the object type out.",
          },
        },
      ],
      "@typescript-eslint/switch-exhaustiveness-check": [
        "error",
        { considerDefaultExhaustiveForUnions: true },
      ],
      "@typescript-eslint/explicit-module-boundary-types": "error",
      // Type aliases are the project's style (the generated bindings are aliases too).
      "@typescript-eslint/consistent-type-definitions": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "no-restricted-syntax": ["error", ...bannedSyntax],
      "no-eval": "error",
      "no-implied-eval": "off",
      "@typescript-eslint/no-implied-eval": "error",
      "no-new-func": "error",
      "no-restricted-globals": [
        "error",
        { name: "localStorage", message: REASON_STORAGE },
        { name: "sessionStorage", message: REASON_STORAGE },
        { name: "indexedDB", message: REASON_STORAGE },
        { name: "caches", message: REASON_STORAGE },
      ],
      "svelte/no-at-html-tags": "error",
      "svelte/block-lang": [
        "error",
        { enforceScriptPresent: false, script: ["ts"] },
      ],
      "svelte/no-inline-styles": ["error", { allowTransitions: false }],
    },
  },
  {
    // A component may import only svelte, other components, the project's own modules and
    // the generated bindings (DESIGN 10): no third-party value can enter untyped.
    files: ["**/*.svelte"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              regex: "^(?!svelte$|svelte/|\\./|\\.\\./)",
              message:
                "A component imports only svelte, components, own modules and the bindings.",
            },
          ],
        },
      ],
    },
  },
  {
    // Generated by ts-rs: held to every type rule, not to the stylistic ones.
    files: ["src/lib/ipc/bindings/**/*.ts"],
    rules: {
      "@typescript-eslint/array-type": "off",
    },
  },
);
