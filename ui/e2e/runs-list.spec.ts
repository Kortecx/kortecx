import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("runs: a submitted run appears in the run list and re-opens", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await runRecipe(page, { fields: { topic: "for the run list" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // POC-5c: run history moved to Monitoring → Runs; the run is listed as a TABLE
  // row (durable ListRuns + session record).
  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible();
  await page.getByTestId("monitor-tab-runs").click();
  await expect(page.getByTestId("monitor-runs")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("run-list")).toContainText("kx/recipes/echo");

  // The filter narrows the list (and an unmatched query empties it).
  await page.getByTestId("runs-filter").fill("echo");
  await expect(page.getByTestId("run-list")).toContainText("kx/recipes/echo");
  await page.getByTestId("runs-filter").fill("zzz-no-match");
  await expect(page.getByTestId("run-list")).toHaveCount(0);

  // Open the run's detail popup (point 4) and re-open the full run from it.
  await page.getByTestId("runs-filter").fill("");
  await page.getByTestId("run-open").first().click();
  await expect(page.getByTestId("run-detail-drawer")).toBeVisible();
  await page.getByTestId("run-view-full").click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
});
