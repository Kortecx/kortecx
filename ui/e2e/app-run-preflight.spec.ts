/**
 * Run preflight (P0): the App Run drawer surfaces feasibility BEFORE firing, so "Run"
 * never looks successful while silently no-op-ing. On a model-free serve it honestly
 * warns that agent steps won't reason — advisory only (the run still fires; the server
 * re-resolves at run).
 */

import { KxClient } from "@kortecx/sdk/node";
import { expect, test } from "@playwright/test";
import { connectConsole, gotoViaPalette } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

const H = "apps/local/preflight-demo";

test("run preflight: warns 'no model served' before firing, but stays advisory", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  const seed = new KxClient(gw.endpoint);
  await seed.saveApp(
    {
      schema: "kortecx.app/v1",
      name: "Preflight Demo",
      blueprint: { seed: 0, steps: [{ kind: "pure", params: { note: "x" } }] },
    },
    { handle: H },
  );
  seed.close();

  await connectConsole(page, gw);
  await gotoViaPalette(page, "apps");
  await expect(page.getByTestId("apps-section")).toBeVisible();
  await page.getByTestId(`app-run-${H}`).click();
  await expect(page.getByTestId("app-run-drawer")).toBeVisible();

  // The model-free serve honestly warns the run won't reason (kills the silent no-op);
  // it must NOT falsely claim "ready".
  await expect(page.getByTestId("app-run-preflight")).toBeVisible();
  await expect(page.getByTestId("app-run-preflight-nomodel")).toBeVisible();
  await expect(page.getByTestId("app-run-preflight-ready")).toHaveCount(0);

  // Advisory only — the run still fires (a pure step commits without a model).
  await expect(page.getByTestId("app-run-now")).toBeVisible();
  await page.getByTestId("app-run-now").click();
  await expect(page).toHaveURL(/\/workflows\//, { timeout: 30_000 });
});
