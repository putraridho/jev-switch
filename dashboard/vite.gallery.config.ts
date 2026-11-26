import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// The component gallery (development and tests only). It has its own entry and output
// directory, so nothing of it can reach the app's `dist` (which the shell embeds).
export default defineConfig({
  plugins: [
    svelte({
      configFile: false,
      compilerOptions: { runes: true },
    }),
  ],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
    host: "localhost",
    open: false,
  },
  build: {
    outDir: "dist-gallery",
    emptyOutDir: true,
    target: "es2022",
    assetsInlineLimit: 0,
    rolldownOptions: { input: "gallery.html" },
  },
});
