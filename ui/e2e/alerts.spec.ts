import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring/alerts: URL-addressable tab, honest 'system healthy' state, Cloud boundary, both themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // A healthy run that COMMITS — it must NOT produce an alert (GR15: the inbox
  // shows only real terminal failures, never a fabricated row).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "ok" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });

  // The Alerts tab is URL-addressable (the run-detail tab precedent).
  await page.getByTestId("monitor-tab-alerts").click();
  await expect(page).toHaveURL(/\/monitor\?tab=alerts$/);
  await expect(page.getByTestId("monitor-tab-alerts")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("monitor-alerts")).toBeVisible();

  // Healthy serve ⇒ the honest "system is healthy" empty state, NOT a spinner or
  // a fabricated row (a committed echo run is not an alert).
  await expect(page.getByText(/System is healthy/).first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("alert-row")).toHaveCount(0);

  // The capability boundary is honest even when healthy: triage/rules/notifications
  // are a Cloud capability (D156) — an honest-disabled card, never a fake control.
  await expect(page.getByTestId("alerts-cloud-disabled")).toBeVisible();

  // BOTH THEMES (D142.1/GR13): the touched view renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.getByText(/System is healthy/).first()).toBeVisible();
  await expect(page.getByTestId("alerts-cloud-disabled")).toBeVisible();

  // Tab click rewrites the URL back to overview (drops the param).
  await page.getByTestId("monitor-tab-overview").click();
  await expect(page).toHaveURL(/\/monitor$/);
});
