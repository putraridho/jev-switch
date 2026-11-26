import { defineConfig, devices } from "@playwright/test";

const port = 4173;
const galleryPort = 4174;

// Screen tests run against the BUILT UI (`pnpm build` first), served with the production
// CSP header by the harness server (docs/dashboard/DESIGN.md 10). Chromium only:
// Playwright's WebKit does not run on Arch.
export default defineConfig({
  testDir: "e2e/screens",
  testMatch: "**/*.spec.ts",
  fullyParallel: true,
  forbidOnly: true,
  retries: 0,
  reporter: "list",
  use: {
    baseURL: `http://127.0.0.1:${String(port)}`,
    trace: "off",
    // The fixtures are written for Asia/Jakarta, 12:33 on Monday 14 Dec 2026.
    timezoneId: "Asia/Jakarta",
    locale: "en-US",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: `node e2e/screens/_harness/server.ts ${String(port)}`,
      url: `http://127.0.0.1:${String(port)}`,
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      // The component gallery: its own build and directory, never part of `dist`.
      command: `pnpm exec vite build -c vite.gallery.config.ts && node e2e/screens/_harness/server.ts ${String(galleryPort)} dist-gallery`,
      url: `http://127.0.0.1:${String(galleryPort)}/gallery.html`,
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
});
