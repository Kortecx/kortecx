/**
 * PR-4.1b: the Workflows + Blueprints card EXPORT affordances download real
 * JSON over the live gateway (the lightweight record, the rich GetProjection/
 * GetContent bundle, and the blueprint definition), and Open-in-new-tab carries
 * `rel="noopener"`. This is the GR17 integration check for the new card menus.
 */

import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows card: Export (lightweight + with-results) downloads run JSON; new-tab is rel=noopener", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "export-me" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });

  const card = page.getByTestId("run-card").first();

  // Lightweight export → a run JSON file.
  await card.getByTestId("run-menu").click();
  const [light] = await Promise.all([
    page.waitForEvent("download"),
    card.getByTestId("run-export").click(),
  ]);
  expect(light.suggestedFilename()).toMatch(/^kortecx-run-.*\.json$/);

  // Rich export → fetches the committed projection + resolved output, then downloads.
  await card.getByTestId("run-menu").click();
  const [rich] = await Promise.all([
    page.waitForEvent("download"),
    card.getByTestId("run-export-rich").click(),
  ]);
  expect(rich.suggestedFilename()).toMatch(/^kortecx-run-.*\.json$/);

  // Open-in-new-tab is a safe external link.
  await card.getByTestId("run-menu").click();
  const newtab = card.getByTestId("run-open-newtab");
  await expect(newtab).toHaveAttribute("target", "_blank");
  await expect(newtab).toHaveAttribute("rel", "noopener noreferrer");
});

test("blueprint card: Export downloads the definition JSON", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-recipes").click();
  await expect(page.getByTestId("recipe-catalog")).toBeVisible({ timeout: 30_000 });

  const bp = page
    .getByTestId("blueprint-card")
    .filter({ has: page.getByTestId("recipe-pick-kx/recipes/echo") });
  await bp.getByTestId("blueprint-menu").click();
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    bp.getByTestId("blueprint-export").click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^kortecx-blueprint-.*\.json$/);
});
