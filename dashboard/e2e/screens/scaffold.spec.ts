// T00's one screen test: the scaffold renders under the production CSP with no request
// outside the app origin. T07's `shell.spec.ts` replaces it.

import { expect, test } from "@playwright/test";
import { guardCsp, guardNetwork, openScreen } from "./_harness/index.ts";

test("the scaffold renders under the production CSP", async ({
  page,
  baseURL,
}) => {
  const violations = guardCsp(page);
  const foreign = guardNetwork(page, baseURL ?? "");
  await openScreen(page, {
    fixture: "pending",
    theme: "light",
    width: 720,
    route: { name: "overview" },
  });
  await expect(
    page.getByRole("heading", { name: "Usage at a glance" }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  expect(violations()).toEqual([]);
  expect(foreign()).toEqual([]);
});
