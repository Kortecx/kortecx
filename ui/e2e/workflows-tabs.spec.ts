/**
 * The /workflows page has a view-toggle: the runnable CATALOG (default) — browse a
 * blueprint (workflow definition) and trigger a single run from its popup — plus your
 * own run HISTORY (Runs) and the self-correction TRAILS. This spec pins the catalog +
 * the definition popup, and that run history is reachable from the Runs tab.
 */

import { expect, test } from "@playwright/test";
import { connectConsole } from "./fixtures/connect";
import { type Gateway, SPA_ORIGIN, spawnGateway } from "./fixtures/serve";

let gw: Gateway | undefined;

test.afterEach(() => {
  gw?.stop();
  gw = undefined;
});

test("workflows page: the runnable catalog + the workflow-definition popup", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();

  // The section has a view-toggle (Catalog / Runs / Trails); the catalog is default.
  await expect(page.getByTestId("workflows-tabs")).toBeVisible();
  await expect(page.getByTestId("workflows-tab-runs")).toBeVisible();
  await expect(page.getByTestId("workflows-list")).toBeVisible({ timeout: 30_000 });
  // The row shows the display NAME now (the raw handle path was dropped from the list);
  // the handle lives on the row's open-button testid.
  await expect(page.getByTestId("workflow-open-kx/recipes/echo")).toBeVisible();

  // Click a workflow row → the definition popup: contract + view + the new-window
  // button (which lives ONLY in the popup).
  await page.getByTestId("workflow-open-kx/recipes/echo").click();
  const drawer = page.getByTestId("workflow-detail-drawer");
  await expect(drawer).toBeVisible();
  // The definition popup lists the workflow's INPUTS (echo has a `topic` param), not raw
  // JSON; the handle still shows in the drawer head; per-item Schedule is offered (a
  // LOCAL CRON trigger — no cloud).
  await expect(drawer.getByTestId("workflow-definition")).toContainText("topic", {
    timeout: 30_000,
  });
  await expect(drawer).toContainText("kx/recipes/echo");
  await expect(drawer.getByTestId("workflow-schedule-kx/recipes/echo")).toBeVisible();
  const newtab = drawer.getByTestId("workflow-open-newtab");
  await expect(newtab).toHaveAttribute("target", "_blank");
  await expect(newtab).toHaveAttribute("rel", "noopener noreferrer");

  // The popup's Run links to the Blueprints run form for that workflow.
  await drawer.getByTestId("workflow-run").click();
  await expect(page).toHaveURL(/\/recipes\?handle=/);
});

test("run history is reachable from the Workflows → Runs tab", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-runs").click();
  await expect(page.getByTestId("runs-section")).toBeVisible();
  await page.getByTestId("workflows-tab-runs").click();
  // The run-history table (RunsTable) lives here now (populated-table assertions
  // live in runs-list.spec.ts, which submits a run first).
  await expect(page.getByTestId("workflows-runs")).toBeVisible({ timeout: 15_000 });
});
