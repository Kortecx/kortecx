/**
 * The Templates tab (App-templates arc): mark a saved App as a template (the reserved
 * `template` tag, persisted via SaveApp), see it in the gallery, then "Use template"
 * to clone it into a new App (CloneApp) and land in the new App's IDE — where the
 * shipped Chat & edit gate enhances it. No new RPC.
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const H = "apps/local/starter";

test("templates: mark an App as a template, then clone it into a new App", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Starter",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "x" } }] },
    },
    { handle: H },
  );
  seed.close();

  await connectConsole(page, gw);
  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();
  await page.getByTestId("workflows-tab-templates").click();
  await expect(page.getByTestId("workflows-templates")).toBeVisible();

  // No templates yet → the honest empty state; the seeded App is markable.
  await expect(page.getByTestId("workflows-templates-placeholder")).toBeVisible();
  await page.getByTestId(`template-mark-${H}`).click();

  // Marked → it appears as a template card (the `template` tag persisted via SaveApp).
  await expect(page.getByTestId(`template-card-${H}`)).toBeVisible({ timeout: 15_000 });

  // Use template → inline name → clone → land in the new App's IDE.
  await page.getByTestId(`template-use-${H}`).click();
  await page.getByTestId(`template-clone-name-${H}`).fill("My Starter Copy");
  await page.getByTestId(`template-clone-submit-${H}`).click();
  await expect(page).toHaveURL(/\/apps\//, { timeout: 30_000 });
  await expect(page.getByTestId("app-detail")).toBeVisible({ timeout: 30_000 });
});
