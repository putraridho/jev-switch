// T13 polish gate: every route in both themes at 320 CSS px and at 200 % zoom (a 640 px
// window, emulated as a 320 CSS px viewport at device pixel ratio 2) has no axe violation,
// no horizontal scroll and no clipped content. Screenshots are the reviewed baselines.

import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import type { Route } from "../../src/lib/ipc/bindings/Route";
import { fragment, themes, type ThemeName } from "./_harness/index.ts";
import { FIXTURE_NOW_MS } from "../../src/lib/ipc/fixtures";

const routes: Route[] = [
  { name: "overview" },
  { name: "provider", provider: "claude" },
  { name: "settings" },
];

const views = [
  { label: "320", width: 320, scale: 1 },
  { label: "zoom200", width: 320, scale: 2 },
] as const;

for (const route of routes) {
  for (const theme of themes) {
    for (const view of views) {
      const name = `${route.name}-${theme}-${view.label}`;
      test(`polish: ${name}`, async ({ browser, baseURL }) => {
        const context = await browser.newContext({
          viewport: { width: view.width, height: 640 },
          deviceScaleFactor: view.scale,
          colorScheme: theme satisfies ThemeName,
          reducedMotion: "reduce",
          timezoneId: "Asia/Jakarta",
          locale: "en-US",
        });
        const page = await context.newPage();
        await page.clock.setFixedTime(FIXTURE_NOW_MS);
        await page.goto(`${baseURL ?? ""}/${fragment(route, theme, "mixed")}`);
        await expect(page.locator("main")).toBeVisible();
        const results = await new AxeBuilder({ page }).analyze();
        expect(results.violations.map((v) => v.id)).toEqual([]);
        const overflow = await page.evaluate(
          () => document.documentElement.scrollWidth - window.innerWidth,
        );
        expect(overflow).toBeLessThanOrEqual(0);
        await expect(page).toHaveScreenshot(`${name}.png`, { fullPage: true });
        await context.close();
      });
    }
  }
}
