/**
 * POC-5c (D168): the /workflows page is now the runnable CATALOG only — browse a
 * blueprint (workflow definition) and trigger a single run from its popup. Run
 * HISTORY moved to Monitoring → Runs (the OSS-Workflows-one-App reframe; multi-app
 * orchestration is Cloud). This spec pins the catalog + the definition popup, and
 * that run history is reachable from Monitoring.
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

  // No view-toggle anymore — the section IS the definitions catalog (rows).
  await expect(page.getByTestId("workflows-tab-runs")).toHaveCount(0);
  await expect(page.getByTestId("workflows-list")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("workflows-list")).toContainText("kx/recipes/echo");

  // Click a workflow row → the definition popup: contract + view + the new-window
  // button (which lives ONLY in the popup).
  await page.getByTestId("workflow-open-kx/recipes/echo").click();
  const drawer = page.getByTestId("workflow-detail-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer.getByTestId("workflow-definition")).toContainText("kx/recipes/echo", {
    timeout: 30_000,
  });
  const newtab = drawer.getByTestId("workflow-open-newtab");
  await expect(newtab).toHaveAttribute("target", "_blank");
  await expect(newtab).toHaveAttribute("rel", "noopener noreferrer");

  // The popup's Run links to the Blueprints run form for that workflow.
  await drawer.getByTestId("workflow-run").click();
  await expect(page).toHaveURL(/\/recipes\?handle=/);
});

test("run history is reachable from Monitoring → Runs (POC-5c move)", async ({ page }) => {
  gw = await spawnGateway({ corsOrigin: SPA_ORIGIN });
  await connectConsole(page, gw);

  await page.getByTestId("nav-monitor").click();
  await expect(page.getByTestId("monitoring-section")).toBeVisible();
  await page.getByTestId("monitor-tab-runs").click();
  // The run-history table (RunsTable) lives here now.
  await expect(page.getByTestId("monitor-runs")).toBeVisible({ timeout: 15_000 });
});
