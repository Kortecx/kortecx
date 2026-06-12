/**
 * PR-2 route merge (D141.1): /workflows is the one home for run telemetry.
 * Covers the sidebar target, the detail tabs (graph/table/artifacts/activity,
 * URL-addressable), and the legacy-path redirects with search preservation
 * (the redirects are route-level, so they assert WITHOUT a live connection).
 */

import { expect, test } from "@playwright/test";
import { connectConsole, runRecipe } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("legacy paths redirect with params + search preserved", async ({ page }) => {
  const instance = "ab".repeat(16); // 32-hex run id
  const ref = "cd".repeat(32); // 64-hex content ref

  await page.goto("/runs");
  await expect(page).toHaveURL(/\/workflows$/);

  await page.goto(`/runs/${instance}?atSeq=3`);
  await expect(page).toHaveURL(new RegExp(`/workflows/${instance}\\?atSeq=3$`));

  await page.goto(`/artifacts?run=${instance}`);
  await expect(page).toHaveURL(new RegExp(`/workflows/${instance}\\?tab=artifacts$`));

  await page.goto(`/artifacts?instance=${instance}&ref=${ref}`);
  await expect(page).toHaveURL(new RegExp(`/workflows/${instance}\\?tab=artifacts&ref=${ref}$`));

  await page.goto("/artifacts");
  await expect(page).toHaveURL(/\/workflows$/);

  await page.goto("/activity");
  await expect(page).toHaveURL(/\/workflows$/);
});

test("workflows: sidebar lands on the run list; the detail tabs are URL-addressable", async ({
  page,
}) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  // The frozen `runs` section id targets /workflows now.
  await page.getByTestId("nav-runs").click();
  await expect(page).toHaveURL(/\/workflows$/);
  await expect(page.getByTestId("runs-section")).toBeVisible();

  // Run a blueprint → its detail page is /workflows/$instanceId.
  await runRecipe(page, { handle: "kx/recipes/fanout-demo" });
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/\/workflows\/[0-9a-f]{32}/);
  await expect
    .poll(() => page.getByTestId("state-pill").filter({ hasText: "COMMITTED" }).count(), {
      timeout: 30_000,
    })
    .toBe(5);

  // Table tab.
  await page.getByTestId("run-tab-table").click();
  await expect(page.getByTestId("mote-table")).toBeVisible();
  await expect(page).toHaveURL(/tab=table/);

  // Artifacts tab — the folded-in gallery (no run picker: the run is in scope).
  await page.getByTestId("run-tab-artifacts").click();
  await expect(page.getByTestId("artifacts-tab")).toBeVisible();
  await expect(page.getByTestId("artifact-gallery")).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/tab=artifacts/);

  // Activity tab — run-scoped metrics + the time-travel scrubber.
  await page.getByTestId("run-tab-activity").click();
  await expect(page.getByTestId("run-activity-tab")).toBeVisible();
  await expect(page.getByTestId("time-travel")).toBeVisible({ timeout: 30_000 });
  await expect(page).toHaveURL(/tab=activity/);

  // Back to the graph (the default tab drops the param).
  await page.getByTestId("run-tab-graph").click();
  await expect(page.getByTestId("mote-dag")).toBeVisible({ timeout: 30_000 });
});
