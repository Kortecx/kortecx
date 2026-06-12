import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring: URL-addressable tabs, the live cross-run feed, telemetry rows + click-through", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // Seed a real run so the feed + telemetry have facts to show.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "tail" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  // The tabs are URL-addressable: clicking writes the validated search param
  // (a full-page deep link reloads the SPA past the connect gate, so the
  // in-session contract is URL-write + history-restore, asserted below).
  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("monitor-tab-feed").click();
  await expect(page).toHaveURL(/\/monitor\?tab=feed$/);
  await expect(page.getByTestId("monitor-tab-feed")).toHaveAttribute("aria-pressed", "true");

  // The global tail streams the run's facts: a run-start row + a committed row,
  // each attributed to its run.
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="run_registered"]').first(),
  ).toBeVisible({ timeout: 20_000 });
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="committed"]').first(),
  ).toBeVisible({ timeout: 20_000 });

  // Click-through: a feed row's run link lands on the run detail — and the
  // history-restored URL re-selects the feed tab (the deep-link read side).
  await page.getByTestId("global-event-run").first().click();
  await expect(page.getByTestId("run-tabs")).toBeVisible({ timeout: 15_000 });
  await page.goBack();
  await expect(page).toHaveURL(/\/monitor\?tab=feed$/);
  await expect(page.getByTestId("monitor-tab-feed")).toHaveAttribute("aria-pressed", "true");

  // The telemetry tab lists the executed mote's exhaust row (joined within the
  // 250 ms tick): wall-clock + seq, with model honestly absent on FFI-free.
  await page.getByTestId("monitor-tab-telemetry").click();
  await expect(page).toHaveURL(/\/monitor\?tab=telemetry$/);
  await expect(page.getByTestId("telemetry-row").first()).toBeVisible({ timeout: 20_000 });

  // Tab clicks rewrite the URL (overview drops the param).
  await page.getByTestId("monitor-tab-overview").click();
  await expect(page).toHaveURL(/\/monitor$/);

  // BOTH THEMES (D142.1/GR13): the touched views render under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await page.getByTestId("monitor-tab-feed").click();
  await expect(page.getByTestId("global-event-row").first()).toBeVisible();
  await page.getByTestId("monitor-tab-telemetry").click();
  await expect(page.getByTestId("telemetry-row").first()).toBeVisible();
});

test("monitoring: honest empty states before any run", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  await page.getByTestId("nav-monitor").click();
  await page.getByTestId("monitor-tab-feed").click();
  // The stream is live but the journal is empty: the listening empty state
  // (actionable copy — D142.3), never a blank panel.
  await expect(page.getByText(/Listening for events|No events yet/).first()).toBeVisible({
    timeout: 15_000,
  });

  await page.getByTestId("monitor-tab-telemetry").click();
  await expect(page.getByText(/No telemetry yet/).first()).toBeVisible({ timeout: 15_000 });
});
