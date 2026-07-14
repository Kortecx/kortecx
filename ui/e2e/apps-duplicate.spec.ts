/**
 * The Apps section "Duplicate" dialog — a portaled, centered modal overlay (not a
 * navbar-clipped inline dialog). This dialog had no e2e coverage; this seeds one App,
 * opens the card kebab → Duplicate, and asserts the dialog renders as a full-viewport
 * overlay ABOVE the sticky navbar (the geometry guard `toBeVisible` alone can't prove),
 * then dismisses on Escape.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { expectOverlayAboveNavbar } from "./fixtures/overlay";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const HANDLE = "apps/local/dup-demo";

test("apps: the Duplicate dialog is a portaled overlay above the navbar", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });

  // Seed one App so its catalog card renders (pure step — model-free serve).
  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Dup Demo",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "demo" } }] },
    },
    { handle: HANDLE },
  );
  seed.close();

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();

  // Card kebab → Duplicate → the centered dialog.
  await page.getByTestId(`app-menu-${HANDLE}`).click();
  await page.getByTestId(`app-duplicate-${HANDLE}`).click();
  await expect(page.getByTestId("app-duplicate-dialog")).toBeVisible();
  await expect(page.getByTestId("app-duplicate-name")).toBeVisible();

  // The dialog is a full-viewport overlay above the sticky navbar (not clipped).
  await expectOverlayAboveNavbar(page, "app-duplicate-dialog");

  // Escape dismisses it.
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("app-duplicate-dialog")).toBeHidden();
});
