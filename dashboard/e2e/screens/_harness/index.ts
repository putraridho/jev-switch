// The Playwright harness for the screen tests (docs/dashboard/DESIGN.md 10): the theme and
// width matrix, the axe runner, the no-network guard and the CSP-violation guard. Outside
// Tauri the built UI runs on a named mock script (`#fixture=...`, `src/lib/ipc/select.ts`),
// and the page clock is frozen at the instant every fixture is written for, so every figure
// and every baseline is reproducible. The time zone and locale come from playwright.config.ts.

import AxeBuilder from "@axe-core/playwright";
import { expect, type Page } from "@playwright/test";
import type { Route } from "../../../src/lib/ipc/bindings/Route";
import {
  FIXTURE_NOW_MS,
  type FixtureName,
} from "../../../src/lib/ipc/fixtures";

/** The two themes every screen is captured in. */
export const themes = ["light", "dark"] as const;
/** A theme of the matrix. */
export type ThemeName = (typeof themes)[number];

/** The three widths (CSS px) every screen is captured at. */
export const widths = [360, 720, 1200] as const;
/** A width of the matrix. */
export type Width = (typeof widths)[number];

/** Which screen to open, in which state. */
export type ScreenOptions = {
  fixture: FixtureName;
  theme: ThemeName;
  width: Width;
  route: Route;
};

/** The URL fragment of DESIGN 7.3 for a route and theme, and the fixture to run on. */
export function fragment(
  route: Route,
  theme: ThemeName,
  fixture: FixtureName = "pending",
): string {
  const view =
    route.name === "provider"
      ? `view=provider&provider=${route.provider}`
      : `view=${route.name}`;
  return `#${view}&theme=${theme}&fixture=${fixture}`;
}

/** Opens the built UI at the given width, theme and route. */
export async function openScreen(
  page: Page,
  options: ScreenOptions,
): Promise<void> {
  await page.setViewportSize({ width: options.width, height: 800 });
  await page.emulateMedia({ colorScheme: options.theme });
  await page.clock.setFixedTime(FIXTURE_NOW_MS);
  await page.goto(
    `/${fragment(options.route, options.theme, options.fixture)}`,
  );
  await page.locator("main").waitFor();
}

/** Fails the test when axe finds a violation of any impact on the page. */
export async function expectNoAxeViolations(page: Page): Promise<void> {
  const results = await new AxeBuilder({ page }).analyze();
  expect(
    results.violations.map(
      (v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`,
    ),
  ).toEqual([]);
}

/** Records every request that is not the app origin; call the result to read them. */
export function guardNetwork(page: Page, origin: string): () => string[] {
  const foreign: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith(origin)) {
      foreign.push(request.url());
    }
  });
  return () => foreign;
}

/** Records every CSP violation the page reports; call the result to read them. */
export function guardCsp(page: Page): () => string[] {
  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy")) {
      violations.push(message.text());
    }
  });
  return () => violations;
}
