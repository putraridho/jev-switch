// T13: the history chart, the diagnostics, the threshold validation, the first-run screen and
// forced colours, in the built UI (DESIGN 9.4, 9.5, 9.6, 9.10). The page clock is frozen at
// 12:33 Monday 14 Dec 2026 and the mock answers with a day of 5-hour usage.

import { expect, test, type Page } from "@playwright/test";
import type { Route } from "../../src/lib/ipc/bindings/Route";
import { type FixtureName } from "../../src/lib/ipc/fixtures";
import {
  expectNoAxeViolations,
  openScreen,
  themes,
  type ThemeName,
} from "./_harness/index.ts";

async function open(
  page: Page,
  route: Route,
  fixture: FixtureName = "known",
  theme: ThemeName = "light",
  width: 360 | 720 | 1200 = 360,
): Promise<void> {
  await openScreen(page, { fixture, theme, width, route });
}

const claude: Route = { name: "provider", provider: "claude" };
const slider = (page: Page) =>
  page.getByRole("slider", { name: "Inspect history" });

test.describe("history chart", () => {
  for (const theme of themes) {
    test(`is drawn, described and passes axe (${theme})`, async ({ page }) => {
      await open(page, claude, "known", theme);
      const image = page.getByRole("img", {
        name: /5-hour usage over the last day/,
      });
      await expect(image).toBeVisible();
      await expect(page.getByText("critical 95%")).toBeVisible();
      await expectNoAxeViolations(page);
      await expect(page.locator("figure")).toHaveScreenshot(
        `chart-${theme}.png`,
      );
    });
  }

  test("the crosshair is a slider the arrow keys, Home and End move", async ({
    page,
  }) => {
    await open(page, claude);
    await slider(page).focus();
    await expect(page.locator(".readout")).toContainText("percent");
    const last = Number(await slider(page).getAttribute("aria-valuemax"));
    await expect(slider(page)).toHaveAttribute("aria-valuenow", String(last));
    await page.keyboard.press("Home");
    await expect(slider(page)).toHaveAttribute("aria-valuenow", "0");
    await page.keyboard.press("ArrowRight");
    await expect(slider(page)).toHaveAttribute("aria-valuenow", "1");
    await expect(slider(page)).toHaveAttribute(
      "aria-valuetext",
      /, \d+ percent$/,
    );
    await page.keyboard.press("End");
    await expect(slider(page)).toHaveAttribute("aria-valuenow", String(last));
    await expect(page.locator("figure")).toHaveScreenshot("chart-inspect.png");
  });

  test("hover shows the same readout and nothing needs a drag", async ({
    page,
  }) => {
    await open(page, claude);
    await slider(page).hover({ position: { x: 120, y: 60 } });
    await expect(page.locator(".readout")).toContainText("percent");
  });

  test("the range is a radio group; choosing one keeps the chart", async ({
    page,
  }) => {
    await open(page, claude);
    const group = page.getByRole("group", { name: "Range" });
    await expect(group.getByRole("radio")).toHaveCount(4);
    await group.getByRole("radio", { name: "Week" }).check();
    await expect(group.getByRole("radio", { name: "Week" })).toBeChecked();
    await expect(
      page.getByRole("img", { name: /over the last week/ }),
    ).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(group.getByRole("radio", { name: "Month" })).toBeChecked();
  });

  test("a meter selector appears for a provider with two meters", async ({
    page,
  }) => {
    await open(page, claude);
    await expect(page.getByLabel("Meter")).toBeVisible();
  });

  test("the data is also a table for a screen reader", async ({ page }) => {
    await open(page, claude);
    await expect(
      page.locator(".visually-hidden table tbody tr").first(),
    ).toBeAttached();
    expect(
      await page.locator(".visually-hidden table tbody tr").count(),
    ).toBeGreaterThan(20);
  });

  test("fits at 320 px with no sideways scroll", async ({ page }) => {
    await open(page, claude);
    await page.setViewportSize({ width: 320, height: 800 });
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test.describe("copy and retry", () => {
  test("Copy puts the command on the clipboard and says Copied", async ({
    page,
    context,
  }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await open(page, { name: "overview" }, "unavailable");
    const copy = page.getByRole("button", { name: /^Copy / }).first();
    await copy.click();
    await expect(
      page.getByRole("button", { name: /^Copied / }).first(),
    ).toBeVisible();
    await expect(page.locator('[aria-live="polite"]')).toHaveText(/Copied/, {
      timeout: 4_000,
    });
  });

  test("a failing card has a Retry that asks again", async ({ page }) => {
    await open(page, { name: "overview" }, "stale_failing");
    await page.getByRole("button", { name: "Retry" }).first().click();
    await expect(page.locator('[aria-live="polite"]')).toHaveText(
      /Checking .* again/,
      {
        timeout: 4_000,
      },
    );
  });
});

test.describe("settings", () => {
  test("a warn level at critical is explained next to the field", async ({
    page,
  }) => {
    await open(page, { name: "settings" });
    const warn = page.getByLabel("Warning (%)");
    await warn.fill("96");
    await warn.blur();
    const message = page.getByText("Warn must be below critical (95)");
    await expect(message).toBeVisible();
    await expect(warn).toHaveAttribute("aria-invalid", "true");
    await expect(warn).toHaveValue("80");
    await expectNoAxeViolations(page);
    await expect(page).toHaveScreenshot("settings-invalid.png", {
      fullPage: true,
    });
  });

  test("the diagnostics open to the version, the sizes and each source", async ({
    page,
  }) => {
    await open(page, { name: "settings" });
    await page.getByText("About and diagnostics").click();
    await expect(
      page.getByText(/Flashboard 0\.1\.0 · history 1\.2 MB/),
    ).toBeVisible();
    await expect(page.getByText("Claude gateway log")).toBeVisible();
    await expect(page.getByText(/last problem: offline/)).toBeVisible();
    await expectNoAxeViolations(page);
    await expect(page).toHaveScreenshot("settings-diagnostics.png", {
      fullPage: true,
    });
  });

  test("turning on the live check asks first, and Escape closes the question", async ({
    page,
  }) => {
    await open(page, { name: "settings" });
    await page.getByRole("switch", { name: /Claude live check/ }).check();
    await expect(
      page.getByRole("dialog", { name: "Turn on the Claude live check?" }),
    ).toBeVisible();
    await expectNoAxeViolations(page);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toBeHidden();
    await expect(
      page.getByRole("switch", { name: /Claude live check/ }),
    ).not.toBeChecked();
    await expect(page).toHaveTitle("Flashboard: Settings");
  });
});

test.describe("first run", () => {
  for (const theme of themes) {
    test(`shows once, passes axe and Continue moves on (${theme})`, async ({
      page,
    }) => {
      await open(page, { name: "overview" }, "first_run", theme);
      await expect(
        page.getByRole("heading", { name: "Usage, in one place." }),
      ).toBeVisible();
      await expect(
        page.getByRole("switch", { name: "Start at login" }),
      ).toBeChecked();
      await expectNoAxeViolations(page);
      await expect(page).toHaveScreenshot(`first-run-${theme}.png`, {
        fullPage: true,
      });
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(
        page.getByRole("heading", { name: "Usage at a glance" }),
      ).toBeFocused();
    });
  }
});

test.describe("forced colours", () => {
  const routes: readonly {
    name: string;
    route: Route;
    fixture: FixtureName;
  }[] = [
    { name: "overview", route: { name: "overview" }, fixture: "mixed" },
    { name: "detail", route: claude, fixture: "known" },
    { name: "settings", route: { name: "settings" }, fixture: "known" },
  ];
  for (const entry of routes) {
    test(`${entry.name} stays usable and legible`, async ({ page }) => {
      await page.emulateMedia({ forcedColors: "active", colorScheme: "light" });
      await openScreen(page, {
        fixture: entry.fixture,
        theme: "light",
        width: 360,
        route: entry.route,
      });
      await page.emulateMedia({ forcedColors: "active" });
      expect(
        await page.evaluate(
          () => matchMedia("(forced-colors: active)").matches,
        ),
      ).toBe(true);
      await expectNoAxeViolations(page);
      // Focus is still visible: the outline is not removed under forced colours.
      await page.keyboard.press("Tab");
      await page.keyboard.press("Tab");
      const outline = await page.evaluate(() => {
        const el = document.activeElement;
        if (el === null) return "";
        const style = getComputedStyle(el);
        return `${style.outlineStyle} ${style.outlineWidth}`;
      });
      expect(outline).not.toMatch(/^none/);
      // Nothing is wider than the window.
      const overflow = await page.evaluate(
        () =>
          document.documentElement.scrollWidth -
          document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
      if (entry.name === "detail") {
        // The chart still draws its line in a forced colour.
        const stroke = await page
          .locator(".line")
          .evaluate((el) => getComputedStyle(el).stroke);
        expect(stroke).not.toBe("none");
      }
    });
  }
});
