// T13: the level-1 disclosure, the keyboard, the live region and focus on navigation, in the
// built UI (DESIGN 9.1, 9.3, 9.10). The page clock is frozen at 12:33 Monday 14 Dec 2026.

import { expect, test, type Page } from "@playwright/test";
import { openScreen } from "./_harness/index.ts";

async function overview(page: Page, fixture: "mixed" | "known" = "mixed") {
  await openScreen(page, {
    fixture,
    theme: "light",
    width: 360,
    route: { name: "overview" },
  });
}

const claudeMeter = (page: Page) =>
  page.getByRole("meter", { name: "Claude 5-hour" });
const liveRegion = (page: Page) => page.locator('[aria-live="polite"]');

test.describe("level 1", () => {
  test("a meter carries the whole detail in its accessible text", async ({
    page,
  }) => {
    await overview(page);
    await expect(claudeMeter(page)).toHaveAttribute(
      "aria-valuetext",
      /^86\.0 percent used, resets in 2 hours 14 minutes, today at 2:47 PM, status warning, seen at 12:32 PM$/,
    );
  });

  test("opens on hover, stays while the pointer is over it, closes when it leaves", async ({
    page,
  }) => {
    await overview(page);
    await claudeMeter(page).hover();
    const detail = page.getByText("86.0 %", { exact: true });
    await expect(detail).toBeVisible();
    await detail.hover();
    await expect(detail).toBeVisible();
    await expect(page).toHaveScreenshot("level1-open-light.png");
    await page.getByRole("heading", { name: "Usage at a glance" }).hover();
    await expect(detail).toBeHidden();
  });

  test("opens on keyboard focus with the same content", async ({ page }) => {
    await overview(page);
    await claudeMeter(page).focus();
    await expect(page.getByText("86.0 %", { exact: true })).toBeVisible();
    for (const text of [
      "Exact reset",
      "today 2:47 PM",
      "Status",
      "Warning",
      "Seen at",
      "12:32 PM",
    ]) {
      await expect(page.getByText(text, { exact: true })).toBeVisible();
    }
  });

  test("Esc closes it without moving focus and without going back", async ({
    page,
  }) => {
    await overview(page);
    await claudeMeter(page).focus();
    await expect(page.getByText("86.0 %", { exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByText("86.0 %", { exact: true })).toBeHidden();
    await expect(claudeMeter(page)).toBeFocused();
    await expect(page).toHaveTitle("Flashboard: Overview");
  });
});

test.describe("keyboard", () => {
  test("the first tab stops are the header, Refresh, then the cards", async ({
    page,
  }) => {
    await overview(page);
    const stops: string[] = [];
    for (let i = 0; i < 5; i += 1) {
      await page.keyboard.press("Tab");
      stops.push(
        await page.evaluate(() => {
          const el = document.activeElement;
          if (el === null) return "";
          return el.getAttribute("aria-label") ?? el.textContent.trim();
        }),
      );
    }
    expect(stops).toEqual([
      "Refresh",
      "Usage",
      "Settings",
      "Claude Max 5x",
      "Claude 5-hour",
    ]);
  });

  test("comma opens Settings, Esc goes back and returns focus", async ({
    page,
  }) => {
    await overview(page);
    await page.keyboard.press(",");
    await expect(page).toHaveTitle("Flashboard: Settings");
    await expect(page.getByRole("heading", { name: "Settings" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page).toHaveTitle("Flashboard: Overview");
    await expect(
      page.getByRole("heading", { name: "Usage at a glance" }),
    ).toBeFocused();
  });

  test("Settings button then Esc puts focus back on the button", async ({
    page,
  }) => {
    await overview(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page).toHaveTitle("Flashboard: Settings");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("button", { name: "Settings" })).toBeFocused();
  });

  test("? opens Settings at Behaviour, which lists every key", async ({
    page,
  }) => {
    await overview(page);
    await page.keyboard.press("?");
    await expect(
      page.getByRole("heading", { name: "Behaviour" }),
    ).toBeFocused();
    for (const key of [
      "R",
      ",",
      "?",
      "1 to 6",
      "Esc",
      "Ctrl and +",
      "Ctrl and -",
      "Ctrl and 0",
    ]) {
      await expect(
        page.locator("kbd", {
          hasText: new RegExp(`^${key.replace(/[+?]/g, "\\$&")}$`),
        }),
      ).toBeVisible();
    }
  });

  test("digits open providers in the fixed order and a hidden provider's digit does nothing", async ({
    page,
  }) => {
    await overview(page);
    await page.keyboard.press("2");
    await expect(page).toHaveTitle("Flashboard: Codex");
    await expect(page.getByRole("heading", { name: "Codex" })).toBeFocused();
    await page.keyboard.press("Escape");
    await page.keyboard.press("3");
    await expect(page).toHaveTitle("Flashboard: DeepSeek");
    await page.keyboard.press("Escape");
    await page.keyboard.press(",");
    await page.getByRole("switch", { name: "Codex", exact: true }).uncheck();
    await page.keyboard.press("Escape");
    await page.keyboard.press("2");
    await expect(page).toHaveTitle("Flashboard: Overview");
  });

  test("R refreshes and says so once", async ({ page }) => {
    await overview(page);
    await page.keyboard.press("r");
    await expect(liveRegion(page)).toHaveText(/^Updated 12:33 PM$/, {
      timeout: 5_000,
    });
  });

  test("single keys do nothing in a text field, and can be switched off", async ({
    page,
  }) => {
    await openScreen(page, {
      fixture: "mixed",
      theme: "light",
      width: 360,
      route: { name: "settings" },
    });
    const warning = page.getByLabel("Warning (%)");
    await warning.focus();
    await page.keyboard.press("r");
    await page.keyboard.press("1");
    await expect(page).toHaveTitle("Flashboard: Settings");
    await page.getByRole("switch", { name: "Single-key shortcuts" }).uncheck();
    await page.getByRole("heading", { name: "Settings" }).focus();
    await page.keyboard.press("1");
    await expect(page).toHaveTitle("Flashboard: Settings");
    await page.keyboard.press("Escape");
    await expect(page).toHaveTitle("Flashboard: Overview");
  });

  test("Ctrl plus, minus and zero move zoom through 100 to 200 in steps of 25", async ({
    page,
  }) => {
    await openScreen(page, {
      fixture: "mixed",
      theme: "light",
      width: 360,
      route: { name: "settings" },
    });
    const zoom = page.getByLabel("Zoom (%)");
    await expect(zoom).toHaveValue("100");
    await page.keyboard.press("Control+Equal");
    await expect(zoom).toHaveValue("125");
    await page.keyboard.press("Control+Equal");
    await expect(zoom).toHaveValue("150");
    await page.keyboard.press("Control+Minus");
    await expect(zoom).toHaveValue("125");
    await page.keyboard.press("Control+Digit0");
    await expect(zoom).toHaveValue("100");
  });
});

test.describe("names", () => {
  test("a card button is named by its visible text, provider first", async ({
    page,
  }) => {
    await overview(page);
    await expect(
      page.getByRole("button", { name: "Claude Max 5x" }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Codex Plus" }),
    ).toBeVisible();
  });

  test("one main landmark, a title per view and a heading per card", async ({
    page,
  }) => {
    await overview(page);
    await expect(page.getByRole("main")).toHaveCount(1);
    await expect(page.getByRole("heading", { level: 1 })).toHaveCount(1);
    await expect(page.getByRole("heading", { level: 2 })).toHaveCount(3);
    await expect(page).toHaveTitle("Flashboard: Overview");
  });
});
