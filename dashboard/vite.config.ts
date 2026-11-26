import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Svelte's options are passed inline: there is no svelte.config file (DESIGN 10).
export default defineConfig({
  plugins: [
    svelte({
      configFile: false,
      compilerOptions: { runes: true },
    }),
  ],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    // Nothing is ever inlined as a `data:` URI: the CSP would block it (DESIGN 8.8).
    assetsInlineLimit: 0,
    sourcemap: false,
  },
});
