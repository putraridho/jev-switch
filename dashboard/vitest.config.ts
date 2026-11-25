import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    svelte({
      configFile: false,
      compilerOptions: { runes: true },
    }),
  ],
  resolve: {
    // Component tests run Svelte's browser build, not its server build.
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
    testTimeout: 60_000,
    hookTimeout: 120_000,
    // Two projects: the unit and component tests run in parallel; the lint self-tests write
    // scratch files into src/ that the project scan would see, so they run one at a time.
    projects: [
      {
        extends: true,
        test: { name: "unit", include: ["src/**/*.test.ts"] },
      },
      {
        extends: true,
        test: {
          name: "scripts",
          include: ["scripts/**/*.test.ts"],
          fileParallelism: false,
        },
      },
    ],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,svelte}"],
      exclude: [
        "src/lib/ipc/bindings/**",
        "src/**/*.test.ts",
        "src/env.d.ts",
        "src/gallery/**",
      ],
      // DESIGN 11.3: `src/lib` logic >= 95 %, components >= 85 % (line coverage).
      thresholds: {
        "src/lib/components/**": { lines: 85 },
        "src/lib/format/**": { lines: 95 },
        "src/lib/stores/**": { lines: 95 },
        "src/lib/router.svelte.ts": { lines: 95 },
        "src/lib/copy.ts": { lines: 95 },
      },
    },
  },
});
