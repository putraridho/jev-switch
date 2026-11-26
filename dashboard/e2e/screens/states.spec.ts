// T13: every screen in every fixture state and both themes has no axe violation of any
// impact (DESIGN 9.10), and the states that matter most are captured as reviewed baselines.

import { expect, test } from "@playwright/test";
import type { Route } from "../../src/lib/ipc/bindings/Route";
import { fixtureNames } from "../../src/lib/ipc/fixtures";
import { expectNoAxeViolations, openScreen, themes } from "./_harness/index.ts";

const routes: readonly Route[] = [
  { name: "overview" },
  { name: "provider", provider: "claude" },
  { name: "provider", provider: "deepseek" },
  { name: "provider", provider: "grok" },
  { name: "settings" },
];

const captured = new Set([
  "overview-mixed",
  "overview-critical",
  "overview-unavailable",
  "overview-elapsed",
  "overview-stale_quiet",
  "provider-claude-known",
  "settings-known",
]);

function label(route: Route): string {
  return route.name === "provider" ? `provider-${route.provider}` : route.name;
}

for (const fixture of fixtureNames) {
  for (const route of routes) {
    for (const theme of themes) {
      const name = `${label(route)}-${fixture}`;
      test(`axe: ${name} ${theme}`, async ({ page }) => {
        await openScreen(page, { fixture, theme, width: 360, route });
        await expectNoAxeViolations(page);
        if (captured.has(name)) {
          await expect(page).toHaveScreenshot(`${name}-${theme}.png`, {
            fullPage: true,
          });
        }
      });
    }
  }
}
