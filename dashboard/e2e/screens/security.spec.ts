// T12: the web view makes no network request and trips no CSP violation (DESIGN 8.10).
// The harness serves the built UI with the exact CSP of tauri.conf.json, so a change that
// needs a remote origin, an inline style or an inline script fails here first.

import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import type { Route } from "../../src/lib/ipc/bindings/Route";
import { guardCsp, guardNetwork, openScreen } from "./_harness/index.ts";

const routes: readonly Route[] = [
  { name: "overview" },
  { name: "provider", provider: "claude" },
  { name: "provider", provider: "codex" },
  { name: "provider", provider: "deepseek" },
  { name: "settings" },
];

for (const route of routes) {
  const label =
    route.name === "provider" ? `provider ${route.provider}` : route.name;
  test(`${label}: no foreign request and no CSP violation`, async ({
    page,
    baseURL,
  }) => {
    const violations = guardCsp(page);
    const foreign = guardNetwork(page, baseURL ?? "");
    // The built entry always talks to the Tauri IPC (there is no browser mock path yet), so
    // the page may log an IPC error in a plain browser; that is not what this test is about.
    await openScreen(page, {
      fixture: "mixed",
      theme: "dark",
      width: 720,
      route,
    });
    await expect(page.locator("main")).toBeVisible();
    await page.waitForLoadState("networkidle");
    expect(foreign()).toEqual([]);
    expect(violations()).toEqual([]);
  });
}

/** Resolves with the violated directive of the first CSP violation the page raises. */
function firstViolation(page: Page, attempt: string): Promise<string> {
  return page.evaluate((kind) => {
    return new Promise<string>((resolve) => {
      document.addEventListener(
        "securitypolicyviolation",
        (event) => {
          resolve(event.violatedDirective);
        },
        { once: true },
      );
      if (kind === "img") {
        const img = document.createElement("img");
        img.src = "https://example.invalid/pixel.png";
        document.body.append(img);
      } else if (kind === "script") {
        const script = document.createElement("script");
        script.src = "https://example.invalid/x.js";
        document.head.append(script);
      } else {
        fetch("https://example.invalid/data").catch(() => {
          // The rejection is the expected outcome; the violation event carries the proof.
        });
      }
    });
  }, attempt);
}

test("the production CSP blocks a remote image, a remote script and a remote fetch", async ({
  page,
}) => {
  await openScreen(page, {
    fixture: "known",
    theme: "light",
    width: 720,
    route: { name: "overview" },
  });
  expect(await firstViolation(page, "img")).toMatch(/^img-src/);
  expect(await firstViolation(page, "script")).toMatch(/^script-src/);
  expect(await firstViolation(page, "fetch")).toMatch(/^connect-src/);
});

test("the document declares no inline style attribute and no inline script", async ({
  page,
}) => {
  await openScreen(page, {
    fixture: "mixed",
    theme: "light",
    width: 720,
    route: { name: "overview" },
  });
  expect(await page.locator("[style]").count()).toBe(0);
  expect(await page.locator("script:not([src])").count()).toBe(0);
});
