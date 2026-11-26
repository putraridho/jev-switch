// The component gallery: a screenshot of each section and a clean axe run, light and dark.
// It is served from its own build (`dist-gallery`), never from the app's `dist`.
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const gallery = "http://127.0.0.1:4174/gallery.html";
const sections = [
  "tokens",
  "buttons",
  "switches",
  "inputs",
  "navigation",
  "overlays",
  "waffle",
  "status",
  "data",
  "table",
  "sparkline",
] as const;

for (const theme of ["light", "dark"] as const) {
  test.describe(`gallery ${theme}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.clock.setFixedTime(new Date("2026-12-14T05:33:00Z"));
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await page.setViewportSize({ width: 820, height: 900 });
      await page.goto(`${gallery}?theme=${theme}`);
      await page.evaluate(() => document.fonts.ready.then(() => undefined));
    });

    test(`sections ${theme}`, async ({ page }) => {
      for (const name of sections) {
        await expect(page.locator(`[data-gallery="${name}"]`)).toHaveScreenshot(
          `gallery-${name}-${theme}.png`,
        );
      }
    });

    test(`axe ${theme}`, async ({ page }) => {
      const result = await new AxeBuilder({ page }).analyze();
      expect(result.violations).toEqual([]);
    });

    test(`axe with menu, tooltip and dialog open ${theme}`, async ({
      page,
    }) => {
      await page.getByRole("button", { name: "Hover or focus" }).focus();
      await expect(page.getByRole("tooltip")).toBeVisible();
      await expect(page.locator('[data-gallery="overlays"]')).toHaveScreenshot(
        `gallery-tooltip-${theme}.png`,
      );
      await page.getByRole("button", { name: "Provider menu" }).click();
      await expect(page.locator('[data-gallery="overlays"]')).toHaveScreenshot(
        `gallery-open-${theme}.png`,
      );
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "Open confirm" }).click();
      await expect(page).toHaveScreenshot(`gallery-dialog-${theme}.png`);
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    });

    test(`dot tooltip ${theme}`, async ({ page }) => {
      const meter = page.getByRole("meter", {
        name: "Claude 5-hour with dot names",
      });
      await meter.focus();
      await expect(page.getByText("Dot 62 of 100")).toBeVisible();
      await expect(
        page.locator('[data-gallery="dot-tooltip"]'),
      ).toHaveScreenshot(`gallery-dot-tooltip-${theme}.png`);
      await page.keyboard.press("Escape");
      await meter.locator('[data-dot="4"]').hover();
      await expect(page.getByText("Dot 5 of 100")).toBeVisible();
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    });

    test(`reflows at 320 px ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 320, height: 800 });
      const wide = await page.evaluate(
        () => document.documentElement.scrollWidth,
      );
      expect(wide).toBeLessThanOrEqual(320);
      await expect(page.locator('[data-gallery="inputs"]')).toHaveScreenshot(
        `gallery-inputs-320-${theme}.png`,
      );
      await expect(page.locator('[data-gallery="table"]')).toHaveScreenshot(
        `gallery-table-320-${theme}.png`,
      );
    });
  });
}
