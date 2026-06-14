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

  // In-app navigate to Runs; the run is listed (durable ListRuns + session record).
  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();
  await expect(page.getByTestId("run-list")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("run-list")).toContainText("kx/recipes/echo");

  // The filter narrows the list (and an unmatched query empties it).
  await page.getByTestId("runs-filter").fill("echo");
  await expect(page.getByTestId("run-list")).toContainText("kx/recipes/echo");
  await page.getByTestId("runs-filter").fill("zzz-no-match");
  await expect(page.getByTestId("run-list")).toHaveCount(0);

  // Re-open the run from its card (clear the filter first).
  await page.getByTestId("runs-filter").fill("");
  await page.getByTestId("run-open").first().click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
});
