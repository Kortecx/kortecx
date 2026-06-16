import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("monitoring/logs: kind filter chips + count badges + free-text filter + NDJSON export, both themes", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // Seed a real run so the global feed has facts to triage.
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "triage" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("monitor-tab-feed").click();
  await expect(page).toHaveURL(/\/monitor\?tab=feed$/);

  // The run's facts stream in: a run-start row + a committed row.
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="run_registered"]').first(),
  ).toBeVisible({ timeout: 20_000 });
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="committed"]').first(),
  ).toBeVisible({ timeout: 20_000 });

  // The W1a-3 triage toolbar is present, with per-kind count badges over the buffer.
  await expect(page.getByTestId("feed-toolbar")).toBeVisible();
  await expect(page.getByTestId("feed-count-committed")).not.toHaveText("0");
  await expect(page.getByTestId("feed-count-run_registered")).not.toHaveText("0");

  // Toggle the committed chip OFF → committed rows hide; run-start rows remain.
  await page.getByTestId("feed-chip-committed").click();
  await expect(page.getByTestId("feed-chip-committed")).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator('[data-testid="global-event-row"][data-kind="committed"]')).toHaveCount(
    0,
  );
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="run_registered"]').first(),
  ).toBeVisible();
  // Toggle it back on → committed rows return.
  await page.getByTestId("feed-chip-committed").click();
  await expect(
    page.locator('[data-testid="global-event-row"][data-kind="committed"]').first(),
  ).toBeVisible();

  // Free-text filter that matches nothing → the honest "no match" state; clearing restores.
  await page.getByTestId("feed-filter").fill("zzzz-no-such-event");
  await expect(page.getByTestId("feed-empty-filtered")).toBeVisible();
  await page.getByTestId("feed-filter").fill("");
  await expect(page.getByTestId("global-event-row").first()).toBeVisible();

  // Export the (filtered) buffer as a download — an .ndjson file (the CLI parity shape).
  const downloadPromise = page.waitForEvent("download");
  await page.getByTestId("feed-export").click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toMatch(/^kortecx-feed-\d+\.ndjson$/);

  // BOTH THEMES (D142.1/GR13): the triage toolbar renders under the dark palette.
  await page.getByTestId("theme-toggle").click();
  await expect(page.getByTestId("feed-toolbar")).toBeVisible();
  await expect(page.getByTestId("feed-chip-failed")).toBeVisible();
});

test("monitoring/telemetry: the token-economy strip is honest-empty on an FFI-free serve", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw, { wsEndpoint: gw.wsEndpoint });

  // A real echo run commits but runs NO model mote (FFI-free) — so there are
  // telemetry rows but ZERO model output tokens. The token-economy strip must
  // show its honest empty state, never a fabricated model/token row (GR15).
  await runRecipe(page, { handle: "kx/recipes/echo", fields: { topic: "econ" } });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("nav-monitor").click();
  await page.getByTestId("monitor-tab-telemetry").click();
  await expect(page).toHaveURL(/\/monitor\?tab=telemetry$/);
  await expect(page.getByTestId("telemetry-row").first()).toBeVisible({ timeout: 20_000 });

  // The cross-page token economy is wired but has no model tokens to show →
  // the honest empty paragraph, and NO fabricated per-model row.
  await expect(page.getByTestId("telemetry-token-economy-empty")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("telemetry-token-economy-row")).toHaveCount(0);
  // The cost tile stays honest-disabled (Cloud-only) regardless.
  await expect(page.getByTestId("cost-tile-disabled")).toBeVisible();
});
