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
    include: ["src/**/*.test.ts", "scripts/**/*.test.ts"],
    testTimeout: 60_000,
    hookTimeout: 120_000,
    // The lint self-test writes scratch files into src/ that the project scan would see.
    fileParallelism: false,
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,svelte}"],
      exclude: ["src/lib/ipc/bindings/**", "src/**/*.test.ts", "src/env.d.ts"],
    },
  },
});
