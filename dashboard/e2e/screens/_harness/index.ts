// The Playwright harness for the screen tests (docs/dashboard/DESIGN.md 10): the theme and
// width matrix, the axe runner, the no-network guard and the CSP-violation guard. The
// exported names and signatures are final; T07 completes the bodies (mock IPC selection,
// the axe run).

import type { Page } from "@playwright/test";
import type { Route } from "../../../src/lib/ipc/bindings/Route";
import type { FixtureName } from "../../../src/lib/ipc/fixtures";

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

/** The URL fragment of DESIGN 7.3 for a route and theme. */
export function fragment(route: Route, theme: ThemeName): string {
  const view =
    route.name === "provider"
      ? `view=provider&provider=${route.provider}`
      : `view=${route.name}`;
  return `#${view}&theme=${theme}`;
}

/** Opens the built UI at the given width, theme and route. */
export async function openScreen(
  page: Page,
  options: ScreenOptions,
): Promise<void> {
  await page.setViewportSize({ width: options.width, height: 800 });
  await page.emulateMedia({ colorScheme: options.theme });
  await page.goto(`/${fragment(options.route, options.theme)}`);
}

/** Fails the test when axe finds a violation on the page. */
export function expectNoAxeViolations(_page: Page): Promise<void> {
  return Promise.resolve();
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
