import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring/cost: URL-addressable tab, honest run-picker (no run → no $), both themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // A committed run so there is real history behind the monitoring surface.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "cost" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });

  // The Cost tab is URL-addressable (the run-detail tab precedent).
  await page.getByTestId("monitor-tab-cost").click();
  await expect(page).toHaveURL(/\/monitor\?tab=cost$/);
  await expect(page.getByTestId("monitor-tab-cost")).toHaveAttribute("aria-pressed", "true");

  // With no run id entered, only the run-picker shows — NO cost card, NO fabricated $
  // (GR15: the cost readout is display-only and never invents a figure).
  await expect(page.getByTestId("cost-run-input")).toBeVisible();
  await expect(page.getByTestId("cost-result")).toHaveCount(0);

  // BOTH THEMES (D142.1/GR13): the touched view renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.getByTestId("cost-run-input")).toBeVisible();

  // The tab param drops back to overview.
  await page.getByTestId("monitor-tab-overview").click();
  await expect(page).toHaveURL(/\/monitor$/);
});
